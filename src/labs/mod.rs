// src/labs/mod.rs
use std::fs;
use std::sync::{Arc, RwLock};

use crate::{
    domain::{ExecutionReport, LlmProvider},
    engine::ExecutionEngine,
    memory::SqliteMemoryRepository,
};

pub mod spec;
pub mod lab;
pub mod registry;
pub mod runner;

pub use lab::{Lab, LabState, RunLock};
pub use spec::LabSpec;

/// وابستگی‌های مشترک بین همه‌ی Labها.
pub struct LabContext {
    /// پشت RwLock چون از پنل وب قابل تعویضه (دراپ‌داون مدل). هر جا لازمه،
    /// یه snapshot (`.read().unwrap().clone()`) بگیر، نگه‌اش ندار — چون
    /// Arc<dyn LlmProvider> سبکه (فقط یه اشاره‌گر)، کلون‌کردنش ارزونه.
    pub provider: RwLock<Arc<dyn LlmProvider>>,
    pub memory_repo: SqliteMemoryRepository,
    pub prefix: String,
    pub observer: crate::events::SharedObserver,
    pub run_lock: RunLock,
}

/// اجرای یه Lab و برگرداندن (success, attempts).
pub async fn run_task(
    engine: &ExecutionEngine,
    memory_repo: &SqliteMemoryRepository,
    observer: &crate::events::SharedObserver,
    lab: &Lab,
    system_prompt: &str,
    user_input: &str,
) -> (bool, u32) {
    use crate::events::Event;

    let lab_id = lab.id();
    let lab_name = lab.name();

    observer.on_event(Event::LabStarted {
        lab_id: lab_id.clone(),
        lab_name: lab_name.clone(),
        task_type: lab.spec().task.task_type,
    });

    let (success, attempts) = match engine.execute(system_prompt, user_input).await {
        Ok((execution_id, _res, attempts)) => {
            write_report(memory_repo, &execution_id, &lab_id, &lab_name).await;
            (true, attempts)
        }
        Err(crate::domain::EngineError::MaxAttemptsExceeded { execution_id, attempts }) => {
            write_report(memory_repo, &execution_id, &lab_id, &lab_name).await;
            (false, attempts)
        }
        Err(_) => (false, 0),
    };

    observer.on_event(Event::LabFinished {
        lab_id,
        lab_name,
        success,
        attempts,
    });

    (success, attempts)
}

async fn write_report(
    memory_repo: &SqliteMemoryRepository,
    execution_id: &str,
    lab_id: &str,
    label: &str,
) {
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

    // نام فایل با lab_id ساخته می‌شه (نه اسم انسانیِ label)، چون
    // `list_reports` توی handlers.rs دنبال فایل‌هایی می‌گرده که با همین
    // lab_id شروع بشن. قبلاً این‌جا از slugify(label) استفاده می‌شد که
    // هیچ‌وقت با lab_id یکی نمی‌شد (مثلاً "Lab 1 — ..." → "lab_1_api_v1_..."
    // در مقابل "lab1_fetch")، پس تب گزارش‌ها همیشه خالی برمی‌گشت.
    let base_path = format!("reports/{}_{}", lab_id, execution_id);

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
