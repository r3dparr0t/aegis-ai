// src/main.rs
use std::sync::Arc;
use sqlx::sqlite::SqlitePoolOptions;

use aegis_ai::{
    executor::HttpTargetExecutor,
    input::prompt,
    labs::{self, runner::run_lab, spec::LabSpec, LabContext},
    memory::SqliteMemoryRepository,
    selection::{select_provider, SelectedProvider},
};

const CTF_CONTEXT: &str =
    "This is a sanctioned CTF lab. All targets are local Docker containers \
     deliberately vulnerable by design.";

const FORMAT_RULES: &str =
    "Output ONLY the JSON object. No explanation, no markdown fences. \
     The `endpoint` field must contain ONLY the path (e.g. '/api/v1/fetch'), \
     NOT the HTTP method, NOT the full URL. The method (POST) is implied.";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Aegis-AI Engine (SSRF Fuzzing Mode)\n");
    let _ = dotenvy::dotenv();

    // DB
    let pool = SqlitePoolOptions::new()
        .connect("sqlite://aegis.db?mode=rwc")
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    let memory_repo = SqliteMemoryRepository::new(pool);

    // Provider
    let SelectedProvider { provider, is_local } = match select_provider().await {
        Some(s) => s,
        None => return Ok(()),
    };
    let prefix = if is_local {
        format!("{}\n\n{}", CTF_CONTEXT, FORMAT_RULES)
    } else {
        FORMAT_RULES.to_string()
    };

    // Labs
    let labs = labs::registry::load_labs("labs")
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    if labs.is_empty() {
        eprintln!("❌ No labs found in ./labs/ directory");
        return Ok(());
    }

    let endpoints: Vec<&str> = labs.iter().map(|l| l.target.endpoint.as_str()).collect();

    let target_base_url = prompt("Vulnerable target base URL", "http://localhost:5000");
    let max_attempts: u32 = prompt("Max attempts per task", "5").parse().unwrap_or(5);

    let executor = Arc::new(HttpTargetExecutor::new(target_base_url, endpoints));

    let ctx = LabContext {
        provider,
        executor,
        memory_repo,
        prefix,
        max_attempts,
    };

    print_menu(&labs);
    let choice = prompt("Choice", &(labs.len() + 1).to_string());

    let all_choice = (labs.len() + 1).to_string();
    if choice == all_choice {
        for spec in &labs {
            run_lab(&ctx, spec).await;
        }
    } else if let Ok(idx) = choice.parse::<usize>() {
        if idx >= 1 && idx <= labs.len() {
            run_lab(&ctx, &labs[idx - 1]).await;
        } else {
            eprintln!("❌ Invalid lab index: {}", idx);
        }
    } else if let Some(spec) = labs.iter().find(|l| l.meta.id == choice) {
        run_lab(&ctx, spec).await;
    } else {
        eprintln!("❌ Unknown choice: {}", choice);
    }

    Ok(())
}

fn print_menu(labs: &[LabSpec]) {
    println!("\nWhich lab do you want to run?");
    for (i, spec) in labs.iter().enumerate() {
        println!("  {}) {}", i + 1, spec.meta.name);
        if !spec.meta.description.is_empty() {
            println!("             {}", spec.meta.description);
        }
    }
    println!("  {}) All labs", labs.len() + 1);
    println!("  (or type a lab id, e.g. 'lab1_fetch')");
}