// src/labs/mod.rs
use std::fs;
use std::sync::Arc;

use crate::{
    domain::{ExecutionReport, LlmProvider},
    engine::ExecutionEngine,
    executor::HttpTargetExecutor,
    memory::SqliteMemoryRepository,
};

pub mod spec;
pub mod registry;
pub mod runner;

pub use spec::LabSpec;

/// وابستگی‌های مشترک بین همه‌ی Labها.
pub struct LabContext {
    pub provider: Arc<dyn LlmProvider>,
    pub executor: Arc<HttpTargetExecutor>,
    pub memory_repo: SqliteMemoryRepository,
    pub prefix: String,
    pub max_attempts: u32,
}

pub async fn run_task(
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
        Err(crate::domain::EngineError::MaxAttemptsExceeded { execution_id, attempts }) => {
            eprintln!("❌ [{}] Execution Failed: exceeded {} attempts", label, attempts);
            write_report(memory_repo, &execution_id, label).await;
        }
        Err(err) => {
            eprintln!("❌ [{}] Execution Failed: {}", label, err);
        }
    }
}

/// یه برچسب انسانی (که ممکنه فاصله، اسلش، خط تیره، پرانتز و ... داشته باشه) رو
/// به یه slug امن برای نام فایل تبدیل می‌کنه. حداکثر ۶۰ کاراکتر.
fn slugify(label: &str) -> String {
    let slug: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();

    // پاک‌سازی underscoreهای پشت‌سرهم و لبه‌ها
    let mut out = String::new();
    let mut last_was_underscore = false;
    for c in slug.chars() {
        if c == '_' {
            if !last_was_underscore && !out.is_empty() {
                out.push('_');
            }
            last_was_underscore = true;
        } else {
            out.push(c);
            last_was_underscore = false;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out.chars().take(60).collect()
}

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

    let base_path = format!("reports/{}_{}", slugify(label), execution_id);

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