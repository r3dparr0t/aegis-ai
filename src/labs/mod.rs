// src/labs/mod.rs
use std::{
    fs,
    sync::{Arc, RwLock, atomic::AtomicBool},
    collections::HashSet,
};

use crate::{
    domain::{ExecutionReport, LlmProvider},
    engine::ExecutionEngine,
    memory::SqliteMemoryRepository,
};

pub mod spec;
pub mod lab;
pub mod registry;
pub mod runner;
pub mod dbox;

pub use lab::{Lab, LabState, RunLock};
pub use spec::LabSpec;

pub struct LabContext {
    pub provider: RwLock<Arc<dyn LlmProvider>>,
    pub memory_repo: SqliteMemoryRepository,
    pub prefix: String,
    pub observer: crate::events::SharedObserver,
    pub run_lock: RunLock,
    pub reports_dir: std::path::PathBuf,
    pub auto_manage_box: Arc<AtomicBool>,
    pub cancel: Arc<AtomicBool>,
    pub started_boxes: Arc<std::sync::Mutex<HashSet<std::path::PathBuf>>>,
}

/// نتیجهٔ واقعیِ یک اجرای engine — سه حالت متمایز که caller باید تفکیک کنه.
#[derive(Debug, Clone, Copy)]
pub enum TaskOutcome {
    Success { attempts: u32 },
    Failed  { attempts: u32 },
    /// کاربر Stop زده — نه success، نه fail. lab باید Cancelled بمونه.
    Cancelled,
    /// خطای غیرمنتظره (LLM/Database) یا اصلاً اجرا شروع نشد.
    Errored,
}

/// اجرای یه Lab و برگرداندن نتیجهٔ کامل.
pub async fn run_task(
    engine: &ExecutionEngine,
    memory_repo: &SqliteMemoryRepository,
    observer: &crate::events::SharedObserver,
    reports_dir: &std::path::Path,
    lab: &Lab,
    system_prompt: &str,
    user_input: &str,
) -> TaskOutcome {
    use crate::domain::EngineError;
    use crate::events::Event;

    let lab_id = lab.id();
    let lab_name = lab.name();

    observer.on_event(Event::LabStarted {
        lab_id: lab_id.clone(),
        lab_name: lab_name.clone(),
        task_type: lab.spec().task.task_type,
    });

    let outcome = match engine.execute(system_prompt, user_input).await {
        Ok((execution_id, _res, attempts)) => {
            write_report(memory_repo, reports_dir, &execution_id, &lab_id, &lab_name).await;
            TaskOutcome::Success { attempts }
        }
        Err(EngineError::MaxAttemptsExceeded { execution_id, attempts }) => {
            write_report(memory_repo, reports_dir, &execution_id, &lab_id, &lab_name).await;
            TaskOutcome::Failed { attempts }
        }
        Err(EngineError::Cancelled { .. }) => TaskOutcome::Cancelled,
        Err(e) => {
            eprintln!("⚠️  Lab '{}' errored: {}", lab_name, e);
            TaskOutcome::Errored
        }
    };

    let (success, attempts) = match outcome {
        TaskOutcome::Success { attempts } => (true, attempts),
        TaskOutcome::Failed  { attempts } => (false, attempts),
        TaskOutcome::Cancelled => (false, 0),
        TaskOutcome::Errored => (false, 0),
    };

    observer.on_event(Event::LabFinished {
        lab_id,
        lab_name,
        success,
        attempts,
    });

    outcome
}

async fn write_report(
    memory_repo: &SqliteMemoryRepository,
    reports_dir: &std::path::Path,
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
    if let Err(e) = fs::create_dir_all(reports_dir) {
        eprintln!("⚠️  Could not create reports directory: {}", e);
        return;
    }

    let base_path = reports_dir.join(format!("{}_{}", lab_id, execution_id));
    match serde_json::to_string_pretty(&report) {
        Ok(json) => {
            if let Err(e) = fs::write(base_path.with_extension("json"), json) {
                eprintln!("⚠️  Could not write JSON report: {}", e);
            }
        }
        Err(e) => eprintln!("⚠️  Could not serialize report to JSON: {}", e),
    }
    let markdown = render_markdown_report(&report, label);
    if let Err(e) = fs::write(base_path.with_extension("md"), markdown) {
        eprintln!("⚠️  Could not write Markdown report: {}", e);
    }
    println!("📄 Report saved: {}.json / {}.md", base_path.display(), base_path.display());
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
        "- **Result:** {}\n",
        if report.success { "✅ SUCCESS" } else { "❌ FAILED" }
    ));
    if report.hint_used {
        md.push_str("- **⚠️ Hint used:** yes — not a clean pass\n");
    }
    if report.fallback_used {
        md.push_str("- **⚠️ Model fallback used:** yes — not solved by the original model alone\n");
    }
    md.push('\n');

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

    if !report.lessons_used.is_empty() {
        let total = report.lessons_used.len();
        let passed = report
            .lessons_used
            .iter()
            .filter(|lu| lu.resulted_in_success)
            .count();
        let failed = total - passed;

        md.push_str("## Lessons Used\n\n");
        md.push_str(&format!(
            "Total: {} lesson-usage record(s) — {} passed, {} failed.\n\n",
            total, passed, failed
        ));
        md.push_str("| Lesson ID | Attempt | Result |\n");
        md.push_str("|-----------|---------|--------|\n");
        for lu in &report.lessons_used {
            let id_short: String = lu.lesson_id.chars().take(8).collect();
            let result = if lu.resulted_in_success { "✅" } else { "❌" };
            md.push_str(&format!(
                "| `{}` | {} | {} |\n",
                id_short, lu.attempt_number, result
            ));
        }
        md.push('\n');
    }

    md
}