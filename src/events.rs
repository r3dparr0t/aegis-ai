// src/events.rs
use std::sync::Arc;
use crate::engine::state::ExecutionState;

/// یک رخداد در طول اجرای engine. GUI و CLI هر دو می‌تونن از این subscribe کنن.
#[derive(Clone, Debug)]
pub enum Event {
    LabStarted { lab_id: String, lab_name: String, task_type: String },
    LabFinished { lab_id: String, lab_name: String, success: bool, attempts: u32 },
    AttemptStarted { execution_id: String, attempt: u32, max_attempts: u32 },
    LessonsInjected { count: usize, texts: Vec<String> },
    /// hintهای دستیِ YAML که این attempt فعال شدن (بعد از after_attempt).
    HintsActive { attempt: u32, messages: Vec<String> },
    /// provider این اجرا، سر یه attempt مشخص، طبق fallback_strategy عوض شد.
    ModelFallback { attempt: u32, from_model: String, to_model: String },
    LlmCalling,
    LlmResponded { raw: String, latency_ms: u64 },
    PayloadParsed { payload: String },
    PayloadTransformed { before: String, after: String, format: String },
    TargetResponded { status: u16, body_excerpt: String, latency_ms: u64 },
    EvaluationPassed,
    EvaluationFailed { category: String, detail: String },
    LessonSaved { id: String, text: String },
    Error {
        context: &'static str,   // "llm" / "database" / "parse" / "execute"
        message: String,
        fatal: bool,
    },
    State { transition: ExecutionState },
}

/// هر کسی که می‌خواد eventها رو ببینه این trait رو پیاده می‌کنه.
pub trait EngineObserver: Send + Sync {
    fn on_event(&self, event: Event);
}

/// پیاده‌سازی پیش‌فرض: همون رفتار قبلی، چاپ به stdout.
pub struct ConsoleObserver;

impl EngineObserver for ConsoleObserver {
    fn on_event(&self, event: Event) {
        match event {
            Event::AttemptStarted { attempt, max_attempts, .. } => {
                println!("\n🔁 [Attempt {}/{}]", attempt, max_attempts);
            }
            Event::LessonsInjected { count, .. } => {
                if count == 0 {
                    println!("🧠 No lessons injected (first attempt or no relevant memory).");
                } else {
                    println!("🧠 Injecting {} lesson(s) into prompt:", count);
                }
            }
            Event::HintsActive { messages, .. } => {
                for m in &messages {
                    println!("💡 Hint active: {}", m);
                }
            }
            Event::ModelFallback { attempt, from_model, to_model } => {
                println!(
                    "🔀 [Attempt {}] Falling back: {} → {}",
                    attempt, from_model, to_model
                );
            }
            Event::LlmCalling => println!("🔷 [STATE] Generating (calling LLM provider)"),
            Event::LlmResponded { raw, .. } => println!("📝 Model raw output:\n{}", raw),
            Event::PayloadParsed { payload } => {
                println!("🔷 [STATE] Executing payload against target: {}", payload);
            }
            Event::PayloadTransformed { before, after, format } => {
                println!("🔧 IP encoding applied: {}", format);
                println!("   before: {}", before);
                println!("   after:  {}", after);
            }
            Event::TargetResponded { status, body_excerpt, .. } => {
                println!("📡 Target responded [{}]: {}", status, body_excerpt);
            }
            Event::EvaluationPassed => println!("✅ Evaluator: PASSED"),
            Event::EvaluationFailed { category, detail } => {
                println!("❌ Evaluator: FAILED [{}] - {}", category, detail);
            }
            Event::LessonSaved { id, text } => {
                println!("💾 Saved new lesson [{}]: {}", &id[..8.min(id.len())], text);
            }
            Event::LabStarted { lab_name, .. } => {
                println!("\n=== 🧪 {} ===", lab_name);
            }
            Event::LabFinished { lab_name, success, attempts, .. } => {
                let v = if success { "✅" } else { "❌" };
                println!("{} [{}] finished after {} attempt(s)", v, lab_name, attempts);
            }
            Event::Error { context, message, fatal } => {
                let tag = if fatal { "💥" } else { "⚠️ " };
                eprintln!("{} [{}] {}", tag, context, message);
            },
            Event::State { transition } => println!("🔷 [STATE] {}", transition),
        }
    }
}

pub type SharedObserver = Arc<dyn EngineObserver>;

