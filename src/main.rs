// src/main.rs
use std::sync::Arc;
use sqlx::sqlite::SqlitePoolOptions;
use aegis_ai::domain::Evaluator;
use aegis_ai::evaluator::JsonEvaluator;
use aegis_ai::memory::SqliteMemoryRepository;
use aegis_ai::provider::OllamaProvider;
use aegis_ai::engine::{EngineConfig, ExecutionEngine};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting Aegis-AI Engine Test...");

    // ۱. مقداردهی پایگاه داده SQLite در حافظه یا فایل
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await?;

    let memory_repo = SqliteMemoryRepository::new(pool);
    memory_repo.init_db().await?;

    // ۲. تعریف Provider و Evaluator
    let provider = Arc::new(OllamaProvider::new("http://localhost:11434", "qwen2.5:3b"));
    let evaluator = Arc::new(JsonEvaluator::new(vec!["status", "target_url"]));

    // ۳. تنظیمات موتور
    let config = EngineConfig {
        max_attempts: 3,
        task_type: "fuzz_target_config".to_string(),
    };

    let engine = ExecutionEngine::new(provider, evaluator, memory_repo, config);

    // ۴. شروع یک کار به شکل عمدی با پرامپت مبهم برای تست Self-Correction
    let system_prompt = "You are an automated fuzzing target generator. Return ONLY JSON.";
    let user_input = "Create a configuration JSON for testing target http://example.com/api";

    println!("⚡ Executing task with Self-Correction loop...");
    match engine.execute(system_prompt, user_input).await {
        Ok(res) => {
            println!("✅ Success!");
            println!("Output:\n{}", res.output);
            println!("Latency: {} ms", res.latency_ms);
        }
        Err(err) => {
            eprintln!("❌ Execution Failed: {}", err);
        }
    }

    Ok(())
}
