// src/main.rs
use std::io::{self, Write};
use std::sync::Arc;
use sqlx::sqlite::SqlitePoolOptions;
use aegis_ai::evaluator::FlagEvaluator;
use aegis_ai::executor::HttpTargetExecutor;
use aegis_ai::memory::SqliteMemoryRepository;
use aegis_ai::provider::OllamaProvider;
use aegis_ai::engine::{EngineConfig, ExecutionEngine};

/// خواندن یک خط از ورودی کاربر، با یک پیام راهنما و مقدار پیش‌فرض در صورت خالی بودن
fn prompt(message: &str, default: &str) -> String {
    if default.is_empty() {
        print!("{} ", message);
    } else {
        print!("{} [{}]: ", message, default);
    }
    io::stdout().flush().ok();

    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();
    let trimmed = input.trim().to_string();

    if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed
    }
}

/// مثل prompt، ولی مقدار خیلی کوتاه رو رد می‌کنه و دوباره می‌پرسه.
/// یه success_marker خیلی کوتاه (مثل یک کاراکتر تنها) می‌تونه به‌طور تصادفی تو هر پاسخی
/// match بشه (false positive)، پس حداقل طول رو اجباری می‌کنیم.
fn prompt_min_len(message: &str, default: &str, min_len: usize) -> String {
    loop {
        let value = prompt(message, default);
        if value.chars().count() >= min_len {
            return value;
        }
        println!(
            "⚠️  Too short ({} char(s)). A marker that short can match unrelated text by accident. Please enter at least {} characters.",
            value.chars().count(),
            min_len
        );
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Aegis-AI Engine (SSRF Fuzzing Mode)\n");

    // ۱. مقداردهی پایگاه داده SQLite دائمی (نه در حافظه) تا Lessonها بین اجراها باقی بمانند
    let pool = SqlitePoolOptions::new()
        .connect("sqlite://aegis.db?mode=rwc")
        .await?;

    // اجرای migrationهای واقعی از پوشه‌ی migrations/ (به‌جای CREATE TABLE IF NOT EXISTS دستی)
    sqlx::migrate!("./migrations").run(&pool).await?;

    let memory_repo = SqliteMemoryRepository::new(pool);

    // ۲. تعریف Provider (مدل)، Executor (هدف واقعی) و Evaluator (پاسخ واقعی هدف)
    // این سه‌تا بین هر دو سناریوی زیر مشترکن؛ فقط task_type و system prompt فرق می‌کنه.
    let ollama_url = prompt("Ollama base URL", "http://localhost:11434");
    let model_name = prompt("Ollama model", "qwen2.5:3b");
    let target_base_url = prompt("Vulnerable target base URL", "http://localhost:5000");
    let success_marker = prompt_min_len("Success marker to look for in target responses", "FLAG{", 4);
    let max_attempts: u32 = prompt("Max attempts per task", "5").parse().unwrap_or(5);

    let provider = Arc::new(OllamaProvider::new(ollama_url, model_name));

    let executor = Arc::new(HttpTargetExecutor::new(
        target_base_url,
        vec!["/api/v1/fetch", "/api/v2/webhook"],
    ));

    let evaluator = Arc::new(FlagEvaluator::new(success_marker));

    println!();
    println!("Which lab do you want to run?");
    println!("  1) Lab 1 - /api/v1/fetch (no filtering)");
    println!("  2) Lab 2 - /api/v2/webhook (naive blacklist on localhost/127.0.0.1)");
    println!("  3) Both");
    let choice = prompt("Choice", "3");

    let run_lab1 = choice == "1" || choice == "3";
    let run_lab2 = choice == "2" || choice == "3";

    // ============================================================
    // آزمایشگاه ۱: /api/v1/fetch — بدون هیچ فیلتری
    // ============================================================
    if run_lab1 {
        println!("\n=== 🧪 Lab 1: /api/v1/fetch (no filtering) ===");

        let default_goal_v1 =
            "Find a way to reach the internal-admin service's /admin/secret-flag endpoint and retrieve the flag using the fetch endpoint.";
        let user_input_v1 = prompt("Goal for Lab 1", default_goal_v1);

        let system_prompt_v1 = r#"You are an automated SSRF exploitation agent testing a lab API.

The target exposes two vulnerable endpoints on the SAME docker network as an internal service called `internal-admin` (port 8080):

1. POST /api/v1/fetch    body: {"url": "<target>"}         -- no filtering at all.
2. POST /api/v2/webhook  body: {"target_url": "<target>"}  -- blocks any string containing "localhost" or "127.0.0.1".

Your job: return ONLY a JSON object shaped like:
{"endpoint": "<one of the two endpoints above>", "body": {<the exact body key that endpoint expects>}}

Your goal is to reach the internal admin service and retrieve its secret flag. Use /api/v1/fetch."#;

        let config1 = EngineConfig {
            max_attempts,
            task_type: "ssrf_internal_admin_v1".to_string(),
        };

        let engine1 = ExecutionEngine::new(
            provider.clone(),
            executor.clone(),
            evaluator.clone(),
            memory_repo.clone(),
            config1,
        );

        run_task(&engine1, "Lab 1", system_prompt_v1, &user_input_v1).await;
    }

    // ============================================================
    // آزمایشگاه ۲: /api/v2/webhook — بلک‌لیست ناشیانه روی "localhost"/"127.0.0.1"
    // task_type متفاوته، پس این سناریو از صفر شروع می‌کنه (بدون Lessonهای آزمایشگاه ۱)
    // ============================================================
    if run_lab2 {
        println!("\n=== 🧪 Lab 2: /api/v2/webhook (naive blacklist on localhost/127.0.0.1) ===");

        let default_goal_v2 =
            "Reach the internal-admin service's /admin/secret-flag endpoint via the webhook endpoint, bypassing its input filter, and retrieve the flag.";
        let user_input_v2 = prompt("Goal for Lab 2", default_goal_v2);

        let system_prompt_v2 = r#"You are an automated SSRF exploitation agent testing a lab API.

There is an internal service called `internal-admin` on port 8080, reachable only from inside the same docker network.

You MUST use exactly this endpoint:
POST /api/v2/webhook   body: {"target_url": "<target>"}

WARNING: this endpoint has an input filter. If your target_url contains the literal substring
"localhost" or "127.0.0.1", the request will be blocked with an error. You must find a way to
reach the internal-admin service WITHOUT using either of those forbidden substrings.

Your job: return ONLY a JSON object shaped like:
{"endpoint": "/api/v2/webhook", "body": {"target_url": "<target>"}}"#;

        let config2 = EngineConfig {
            max_attempts,
            task_type: "ssrf_webhook_blacklist".to_string(),
        };

        let engine2 = ExecutionEngine::new(provider, executor, evaluator, memory_repo, config2);

        run_task(&engine2, "Lab 2", system_prompt_v2, &user_input_v2).await;
    }

    Ok(())
}

/// اجرای یک سناریو و چاپ نتیجه‌ی نهایی‌اش
async fn run_task(engine: &ExecutionEngine, label: &str, system_prompt: &str, user_input: &str) {
    println!("⚡ [{}] Executing SSRF self-correction loop...", label);
    match engine.execute(system_prompt, user_input).await {
        Ok(res) => {
            println!("✅ [{}] Success! Target responded with the success marker.", label);
            println!("Target response:\n{}", res.output);
            println!("Latency: {} ms", res.latency_ms);
        }
        Err(err) => {
            eprintln!("❌ [{}] Execution Failed: {}", label, err);
        }
    }
}
