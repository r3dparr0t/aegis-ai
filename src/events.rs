// src/events.rs
use std::sync::Arc;
use crate::engine::state::ExecutionState;

#[derive(Clone, Debug)]
pub enum Event {
    LabStarted { lab_id: String, lab_name: String, task_type: String },
    LabFinished { lab_id: String, lab_name: String, success: bool, attempts: u32 },
    AttemptStarted { execution_id: String, attempt: u32, max_attempts: u32 },
    LessonsInjected { count: usize, texts: Vec<String> },
    HintsActive { attempt: u32, messages: Vec<String> },
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
        context: &'static str,
        message: String,
        fatal: bool,
    },
    State { transition: ExecutionState },

    // ─── Docker/box lifecycle ───
    BoxUp      { compose_dir: String },
    BoxDown    { compose_dir: String },
    /// شروع یک دستور docker (up / down / stop / exec ...)
    BoxStarting { compose_dir: String, action: String },
    /// یک خط خروجی از docker (stdout یا stderr). ANSI codes قبلاً strip شدن.
    BoxOutput   { compose_dir: String, stream: String, line: String },
    /// پایان دستور docker با success / exit code.
    BoxFinished { compose_dir: String, action: String, success: bool },

    SetupCommandStarted  { service: String },
    SetupCommandFinished { service: String, success: bool, excerpt: String },

    ServerUp   { lab_id: String },
    ServerDown { lab_id: String, reason: String },
    LabSkipped { lab_id: String, reason: String },
}

pub trait EngineObserver: Send + Sync {
    fn on_event(&self, event: Event);
}

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
                for m in &messages { println!("💡 Hint active: {}", m); }
            }
            Event::ModelFallback { attempt, from_model, to_model } => {
                println!("🔀 [Attempt {}] Falling back: {} → {}", attempt, from_model, to_model);
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
                let short: String = id.chars().take(8).collect();
                println!("💾 Saved new lesson [{}]: {}", short, text);
            }
            Event::LabStarted { lab_name, .. } => println!("\n=== 🧪 {} ===", lab_name),
            Event::LabFinished { lab_name, success, attempts, .. } => {
                let v = if success { "✅" } else { "❌" };
                println!("{} [{}] finished after {} attempt(s)", v, lab_name, attempts);
            }
            Event::Error { context, message, fatal } => {
                let tag = if fatal { "💥" } else { "⚠️ " };
                eprintln!("{} [{}] {}", tag, context, message);
            },
            Event::State { transition } => println!("🔷 [STATE] {}", transition),
            Event::BoxUp { compose_dir }   => println!("🐳 box up: {}", compose_dir),
            Event::BoxDown { compose_dir } => println!("🐳 box down: {}", compose_dir),
            Event::BoxStarting { action, compose_dir } => {
                println!("🐳 → docker {} in {}", action, compose_dir);
            }
            Event::BoxOutput { stream, line, .. } => {
                let prefix = if stream == "stderr" { "stderr" } else { "stdout" };
                println!("   [{}] {}", prefix, line);
            }
            Event::BoxFinished { action, success, .. } => {
                let v = if success { "✓" } else { "✗" };
                println!("🐳 {} docker {} finished", v, action);
            }
            Event::SetupCommandStarted { service } => {
                println!("🔧 setup: running command in '{}'...", service);
            }
            Event::SetupCommandFinished { service, success, excerpt } => {
                let mark = if success { "✓" } else { "✗" };
                println!("🔧 setup [{}] {}: {}", service, mark, excerpt);
            }
            Event::ServerUp { lab_id } => println!("✓ server up for '{}' — going to fuzz", lab_id),
            Event::ServerDown { lab_id, reason } => {
                println!("✗ server down for '{}': {}", lab_id, reason);
            }
            Event::LabSkipped { lab_id, reason } => {
                println!("⏭  Lab '{}' skipped: {}", lab_id, reason);
            }
        }
    }
}

pub type SharedObserver = Arc<dyn EngineObserver>;