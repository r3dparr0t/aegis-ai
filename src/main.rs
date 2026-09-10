// src/main.rs
use std::sync::Arc;
use sqlx::sqlite::SqlitePoolOptions;
use aegis_ai::evaluator::FlagEvaluator;
use aegis_ai::executor::HttpTargetExecutor;
use aegis_ai::memory::SqliteMemoryRepository;
use aegis_ai::provider::OllamaProvider;
use aegis_ai::engine::{EngineConfig, ExecutionEngine};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting Aegis-AI Engine (SSRF Fuzzing Mode)...");

    // ۱. مقداردهی پایگاه داده SQLite دائمی (نه در حافظه) تا Lessonها بین اجراها باقی بمانند
    let pool = SqlitePoolOptions::new()
        .connect("sqlite://aegis.db?mode=rwc")
        .await?;

    let memory_repo = SqliteMemoryRepository::new(pool);
    memory_repo.init_db().await?;

    // ۲. تعریف Provider (مدل)، Executor (هدف واقعی) و Evaluator (پاسخ واقعی هدف)
    let provider = Arc::new(OllamaProvider::new("http://localhost:11434", "qwen2.5:3b"));

    let executor = Arc::new(HttpTargetExecutor::new(
        "http://localhost:5000",
        vec!["/api/v1/fetch", "/api/v2/webhook"],
    ));

    // موفقیت یعنی رسیدن payload به internal-admin و دیدن فلگ در پاسخ واقعی هدف
    let evaluator = Arc::new(FlagEvaluator::new("FLAG{"));

    // ۳. تنظیمات موتور
    let config = EngineConfig {
        max_attempts: 5,
        task_type: "ssrf_internal_admin".to_string(),
    };

    let engine = ExecutionEngine::new(provider, executor, evaluator, memory_repo, config);

    // ۴. توضیح دقیق دو endpoint آسیب‌پذیر و schema مورد انتظار Executor به مدل
    let system_prompt = r#"You are an automated SSRF exploitation agent testing a lab API.

The target exposes two vulnerable endpoints on the SAME docker network as an internal service called `internal-admin` (port 8080):

1. POST /api/v1/fetch    body: {"url": "<target>"}         -- no filtering at all.
2. POST /api/v2/webhook  body: {"target_url": "<target>"}  -- blocks any string containing "localhost" or "127.0.0.1".

Your job: return ONLY a JSON object shaped like:
{"endpoint": "<one of the two endpoints above>", "body": {<the exact body key that endpoint expects>}}

Your goal is to reach the internal admin service and retrieve its secret flag. Do not use "localhost" or "127.0.0.1" — use the internal docker service name instead."#;

    let user_input = "Find a way to reach the internal-admin service's /admin/secret-flag endpoint and retrieve the flag.";

    println!("⚡ Executing SSRF self-correction loop...");
    match engine.execute(system_prompt, user_input).await {
        Ok(res) => {
            println!("✅ Success! Target responded with the success marker.");
            println!("Target response:\n{}", res.output);
            println!("Latency: {} ms", res.latency_ms);
        }
        Err(err) => {
            eprintln!("❌ Execution Failed: {}", err);
        }
    }

    Ok(())
}
