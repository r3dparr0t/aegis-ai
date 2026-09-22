// src/main.rs

use std::{fs, sync::Arc};
use sqlx::sqlite::SqlitePoolOptions;
use uuid::Uuid;

use aegis_ai::{
    domain::{ExecutionReport, LlmProvider},
    evaluator::FlagEvaluator,
    executor::HttpTargetExecutor,
    input::{prompt, prompt_min_len},   
    memory::SqliteMemoryRepository,
    provider::OllamaProvider,
    selection::{choose_ollama_model, select_provider},
    engine::{EngineConfig, ExecutionEngine},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Aegis-AI Engine (SSRF Fuzzing Mode)\n");
    // .env رو قبل از هر چیزی لود کن
    let _ = dotenvy::dotenv();

    // ۱. مقداردهی پایگاه داده SQLite دائمی (نه در حافظه) تا Lessonها بین اجراها باقی بمانند
    let pool = SqlitePoolOptions::new()
        .connect("sqlite://aegis.db?mode=rwc")
        .await?;

    // اجرای migrationهای واقعی از پوشه‌ی migrations/ (به‌جای CREATE TABLE IF NOT EXISTS دستی)
    sqlx::migrate!("./migrations").run(&pool).await?;

    let memory_repo = SqliteMemoryRepository::new(pool);


     // ۲. انتخاب provider — همه‌ی منطق تو selection.rs
    let provider: Arc<dyn LlmProvider> = match select_provider().await {
        Some(p) => p,
        None => return Ok(()),
    };


    let target_base_url = prompt("Vulnerable target base URL", "http://localhost:5000");
    let success_marker = prompt_min_len("Success marker to look for in target responses", "FLAG{", 4);
    let max_attempts: u32 = prompt("Max attempts per task", "5").parse().unwrap_or(5);

    let executor = Arc::new(HttpTargetExecutor::new(
        target_base_url,
        vec!["/api/v1/fetch", "/api/v2/webhook", "/api/v3/strict"],
    ));

    let evaluator = Arc::new(FlagEvaluator::new(success_marker));

    println!("\nWhich lab do you want to run?");
    println!("  1) Lab 1 - /api/v1/fetch (no filtering)");
    println!("  2) Lab 2 - /api/v2/webhook (naive blacklist on localhost/127.0.0.1)");
    println!("  3) Lab 3 - memory isolation test (fresh task_type every run;");
    println!("             attempt 1 MUST show zero injected lessons, even though");
    println!("             Lab 1/Lab 2 already have lessons in aegis.db from earlier runs)");
    println!("  4) Lab 4 - /api/v3/strict, blocks hostname AND dotted-quad IP notation;");
    println!("             only a real IP-encoding bypass (decimal/hex integer) works");
    println!("  5) All four");
    let choice = prompt("Choice", "5");

    let run_lab1 = choice == "1" || choice == "5";
    let run_lab2 = choice == "2" || choice == "5";
    let run_lab3 = choice == "3" || choice == "5";
    let run_lab4 = choice == "4" || choice == "5";

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

        run_task(&engine1, &memory_repo, "Lab 1", system_prompt_v1, &user_input_v1).await;
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

        let engine2 = ExecutionEngine::new(
            provider.clone(),
            executor.clone(),
            evaluator.clone(),
            memory_repo.clone(),
            config2,
        );

        run_task(&engine2, &memory_repo, "Lab 2", system_prompt_v2, &user_input_v2).await;
    }

    // ============================================================
    // آزمایشگاه ۳: تست ایزوله‌بودن حافظه — یک task_type کاملاً تازه در هر اجرا
    // (با یک UUID تصادفی)، تا مطمئن شویم Lessonهای Lab 1/Lab 2 اینجا نشت نمی‌کنن.
    // اگه fetch_lessons درست کار کنه، Attempt 1 باید "No lessons injected" بزنه،
    // حتی با اینکه aegis.db از اجراهای قبلی پر از Lesson برای Lab 1 و Lab 2 هست.
    // ============================================================
    if run_lab3 {
        let fresh_task_type = format!("ssrf_isolation_test_{}", &Uuid::new_v4().simple().to_string()[..8]);
        println!("\n=== 🧪 Lab 3: memory isolation test (task_type={}) ===", fresh_task_type);
        println!("👀 Watch Attempt 1 below: it MUST say 'No lessons injected', proving Lab 1/Lab 2 lessons don't leak here.");

        let default_goal_v3 =
            "Find a way to reach the internal-admin service's /admin/secret-flag endpoint and retrieve the flag using the fetch endpoint.";
        let user_input_v3 = prompt("Goal for Lab 3", default_goal_v3);

        // همون سناریوی Lab 1 (تا اگه port confusion دوباره رخ داد، بتونیم مقایسه کنیم)
        let system_prompt_v3 = r#"You are an automated SSRF exploitation agent testing a lab API.

The target exposes two vulnerable endpoints on the SAME docker network as an internal service called `internal-admin` (port 8080):

1. POST /api/v1/fetch    body: {"url": "<target>"}         -- no filtering at all.
2. POST /api/v2/webhook  body: {"target_url": "<target>"}  -- blocks any string containing "localhost" or "127.0.0.1".

Your job: return ONLY a JSON object shaped like:
{"endpoint": "<one of the two endpoints above>", "body": {<the exact body key that endpoint expects>}}

Your goal is to reach the internal admin service and retrieve its secret flag. Use /api/v1/fetch."#;

        let config3 = EngineConfig {
            max_attempts,
            task_type: fresh_task_type,
        };

        let engine3 = ExecutionEngine::new(
            provider.clone(),
            executor.clone(),
            evaluator.clone(),
            memory_repo.clone(),
            config3,
        );

        run_task(&engine3, &memory_repo, "Lab 3", system_prompt_v3, &user_input_v3).await;
    }

    // ============================================================
    // آزمایشگاه ۴: /api/v3/strict — بلک‌لیست هم نام‌میزبان (internal-admin) هم نماد
    // dotted-quad IP رو می‌گیره. تنها راه عبور واقعی، انکود کردن IP به فرمت decimal/hex
    // بدون نقطه‌ست (که resolver سیستم عامل هنوز به‌عنوان IP معتبر قبولش می‌کنه).
    // IP ثابت internal-admin (172.28.0.10) تو docker-compose.yml پین شده تا این چالش
    // بین rebuildها عوض نشه.
    // ============================================================
    if run_lab4 {
        println!("\n=== 🧪 Lab 4: /api/v3/strict (blocks hostname + dotted-quad IP notation) ===");

        // این چالش نیاز به یک محاسبه‌ی چندمرحله‌ای (تبدیل مبنا) داره که مدل‌های کوچیک
        // (مثل qwen2.5:3b) معمولاً درش شکست می‌خورن (حدس می‌زنن، محاسبه نمی‌کنن).
        // این‌جا اجازه می‌دیم یک مدل جداگانه (بزرگ‌تر) فقط برای همین Lab انتخاب بشه.
        let use_bigger_model = prompt(
            "Use a different (bigger) Ollama model just for Lab 4's IP-encoding arithmetic? [y/N]",
            "n",
        );

        let engine4_provider: Arc<dyn LlmProvider> = if use_bigger_model.eq_ignore_ascii_case("y") {
            let lab4_ollama_url = prompt("Ollama base URL for Lab 4", "http://localhost:11434");
            let lab4_model = choose_ollama_model(&lab4_ollama_url, "qwen2.5:7b").await;
            Arc::new(OllamaProvider::new(lab4_ollama_url, lab4_model))
        } else {
            provider.clone()
        };

        let default_goal_v4 =
            "Reach the internal-admin service at 172.28.0.10:8080's /admin/secret-flag endpoint via the strict endpoint, bypassing its filter with an IP-encoding technique, and retrieve the flag.";
        let user_input_v4 = prompt("Goal for Lab 4", default_goal_v4);

        let system_prompt_v4 = r#"You are an automated SSRF exploitation agent testing a lab API.

There is an internal service called `internal-admin` at IP address 172.28.0.10, port 8080,
reachable only from inside the same docker network.

You MUST use exactly this endpoint:
POST /api/v3/strict   body: {"target_url": "<target>"}

WARNING: this endpoint has a strict input filter. It blocks:
- the literal substrings "localhost", "127.0.0.1", and "internal-admin"
- any URL containing dotted-quad IP notation (e.g. "172.28.0.10" will be blocked outright)

You must reach 172.28.0.10:8080 WITHOUT using its dotted-quad form or any forbidden hostname.

Hint: the underlying HTTP client resolves numeric hostnames using legacy IP encodings. You can
express an IPv4 address as a single 32-bit decimal integer (combine the four octets: the address
a.b.c.d equals a*16777216 + b*65536 + c*256 + d) or as a hex integer prefixed with 0x. Compute the
correct encoded form of 172.28.0.10 and use it as the host in your target_url, keeping :8080 and
the path.

Your job: return ONLY a JSON object shaped like:
{"endpoint": "/api/v3/strict", "body": {"target_url": "<target>"}}"#;

        let config4 = EngineConfig {
            max_attempts,
            task_type: "ssrf_ip_encoding_bypass".to_string(),
        };

        let engine4 = ExecutionEngine::new(engine4_provider, executor, evaluator, memory_repo.clone(), config4);

        run_task(&engine4, &memory_repo, "Lab 4", system_prompt_v4, &user_input_v4).await;
    }

    Ok(())
}

/// اجرای یک سناریو، چاپ نتیجه‌ی نهایی، و تولید گزارش ساختاریافته (JSON + Markdown)
/// چه در صورت موفقیت، چه در صورت شکست پس از اتمام حداکثر تلاش‌ها.
async fn run_task(
    engine: &ExecutionEngine,
    memory_repo: &SqliteMemoryRepository,
    label: &str,
    system_prompt: &str,
    user_input: &str,
) {
    println!("⚡ [{}] Executing SSRF self-correction loop...", label);
    match engine.execute(system_prompt, user_input).await {
        Ok((execution_id, res)) => {
            println!("✅ [{}] Success! Target responded with the success marker.", label);
            println!("Target response:\n{}", res.output);
            println!("Latency: {} ms", res.latency_ms);
            write_report(memory_repo, &execution_id, label).await;
        }
        Err(aegis_ai::domain::EngineError::MaxAttemptsExceeded { execution_id, attempts }) => {
            eprintln!("❌ [{}] Execution Failed: exceeded {} attempts", label, attempts);
            write_report(memory_repo, &execution_id, label).await;
        }
        Err(err) => {
            // این شاخه‌ها (خطای LLM/DB) قبل یا بدون تکمیل یک execution رخ می‌دن،
            // پس چیزی برای گزارش‌گیری از جدول attempts وجود نداره.
            eprintln!("❌ [{}] Execution Failed: {}", label, err);
        }
    }
}

/// بازسازی گزارش از SQLite و نوشتن آن به‌صورت JSON و Markdown در پوشه‌ی reports/
async fn write_report(memory_repo: &SqliteMemoryRepository, execution_id: &str, label: &str) {
    let report = match memory_repo.get_execution_report(execution_id).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("⚠️  Could not build report for {}: {}", label, e);
            return;
        }
    };

    if let Err(e) = fs::create_dir_all("reports") {
        eprintln!("⚠️  Could not create reports/ directory: {}", e);
        return;
    }

    let base_path = format!("reports/{}_{}", label.to_lowercase().replace(' ', "_"), execution_id);

    match serde_json::to_string_pretty(&report) {
        Ok(json) => {
            if let Err(e) = fs::write(format!("{}.json", base_path), json) {
                eprintln!("⚠️  Could not write JSON report: {}", e);
            }
        }
        Err(e) => eprintln!("⚠️  Could not serialize report to JSON: {}", e),
    }

    let markdown = render_markdown_report(&report, label);
    if let Err(e) = fs::write(format!("{}.md", base_path), markdown) {
        eprintln!("⚠️  Could not write Markdown report: {}", e);
    }

    println!("📄 Report saved: {}.json / {}.md", base_path, base_path);
}

/// تبدیل ExecutionReport به یک سند Markdown خوانا
fn render_markdown_report(report: &ExecutionReport, label: &str) -> String {
    let mut md = String::new();

    md.push_str(&format!("# Aegis-AI Execution Report — {}\n\n", label));
    md.push_str(&format!("- **Execution ID:** {}\n", report.execution_id));
    md.push_str(&format!("- **Task type:** {}\n", report.task_type));
    md.push_str(&format!("- **Goal:** {}\n", report.goal));
    md.push_str(&format!("- **Created at:** {}\n", report.created_at));
    md.push_str(&format!("- **Attempts:** {}\n", report.attempts.len()));
    md.push_str(&format!(
        "- **Result:** {}\n\n",
        if report.success { "✅ SUCCESS" } else { "❌ FAILED" }
    ));

    md.push_str("## Attempts\n\n");

    for attempt in &report.attempts {
        let verdict = if attempt.is_valid { "✅ PASSED" } else { "❌ FAILED" };
        md.push_str(&format!("### Attempt {} — {}\n\n", attempt.attempt_number, verdict));

        if let Some(ref cat) = attempt.error_category {
            md.push_str(&format!("- **Error category:** {}\n", cat));
        }
        if let Some(ref details) = attempt.error_details {
            md.push_str(&format!("- **Error details:** {}\n", details));
        }
        md.push_str(&format!("- **Latency:** {} ms\n\n", attempt.latency_ms));

        md.push_str("**Output:**\n\n```\n");
        md.push_str(&attempt.output);
        md.push_str("\n```\n\n");
    }

    md
}
