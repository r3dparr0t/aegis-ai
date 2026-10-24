// src/web/state.rs
use std::{collections::HashMap, sync::{Arc, Mutex}, sync::atomic::AtomicBool};
use serde::Serialize;
use crate::{
    events::{EngineObserver, Event},
    labs::RunLock,
};

#[derive(Clone, Debug, Serialize)]
pub struct LogLine {
    pub time: u64,
    pub icon: String,
    pub text: String,
    pub kind: String,
}

pub struct WebState {
    pub log: Vec<LogLine>,
    pub server_up: HashMap<String, bool>,

    // ★ Settings — منبع حقیقت runtime
    pub auto_manage_box: Arc<AtomicBool>,
    pub host: String,
    pub port: u16,
    pub boot_host: String,
    pub boot_port: u16,
}

impl WebState {
    pub fn new(auto_manage_box: Arc<AtomicBool>, host: String, port: u16) -> Self {
        Self {
            log: Vec::new(),
            server_up: HashMap::new(),
            auto_manage_box,
            host: host.clone(),
            port,
            boot_host: host,
            boot_port: port,
        }
    }

    pub fn push(&mut self, icon: &str, text: impl Into<String>, kind: &str) {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64).unwrap_or(0);
        self.log.push(LogLine { time: t, icon: icon.into(), text: text.into(), kind: kind.into() });
        if self.log.len() > 400 { self.log.drain(0..100); }
    }
}

impl Default for WebState {
    fn default() -> Self {
        Self::new(Arc::new(AtomicBool::new(true)), "127.0.0.1".to_string(), 7777)
    }
}

pub type SharedWebState = Arc<Mutex<WebState>>;

pub struct WebStateObserver {
    pub state: SharedWebState,
    pub run_lock: RunLock,
}

// ★★★ بدنه‌ی on_event دقیقاً مثل قبل — دست‌نخورده.
impl EngineObserver for WebStateObserver {
    fn on_event(&self, event: Event) {
        // ... همون کد قبلی، بدون هیچ تغییری ...
        // (کل match از Event::LabStarted تا Event::LabSkipped را عیناً نگه دار)
        let mut s = self.state.lock().unwrap();
        match &event {
            Event::LabStarted { .. } => {}
            Event::AttemptStarted { attempt, max_attempts, .. } => {
                if let Some(lab) = self.run_lock.lock().unwrap().as_ref() {
                    lab.set_progress(*attempt, *max_attempts);
                }
                s.push("🔁", format!("Attempt {}/{}", attempt, max_attempts), "state");
            }
            Event::LessonsInjected { count, .. } => {
                if *count > 0 { s.push("🧠", format!("{} lesson(s)", count), "info"); }
            }
            Event::HintsActive { messages, .. } => {
                for m in messages { s.push("💡", m.chars().take(120).collect::<String>(), "info"); }
            }
            Event::ModelFallback { attempt, from_model, to_model } => {
                s.push("🔀", format!("fallback (attempt {}): {} → {}", attempt, from_model, to_model), "state");
            }
            Event::LlmResponded { raw, latency_ms } => {
                s.push("📥", format!("LLM {}ms: {}", latency_ms, raw.chars().take(80).collect::<String>()), "info");
            }
            Event::PayloadParsed { payload } => {
                s.push("🎯", payload.chars().take(120).collect::<String>(), "info");
            }
            Event::PayloadTransformed { format, after, .. } => {
                s.push("🔧", format!("{} → {}", format, after.chars().take(80).collect::<String>()), "info");
            }
            Event::TargetResponded { status, latency_ms, .. } => {
                s.push("📡", format!("HTTP {} · {}ms", status, latency_ms), "info");
            }
            Event::EvaluationPassed => s.push("✅", "PASSED", "pass"),
            Event::EvaluationFailed { category, .. } => s.push("❌", format!("FAILED [{}]", category), "fail"),
            Event::LessonSaved { text, .. } => s.push("💾", text.chars().take(100).collect::<String>(), "info"),
            Event::LabFinished { success, attempts, .. } => {
                let v = if *success { "✅" } else { "❌" };
                s.push(v, format!("finished after {} attempt(s)", attempts), "state");
            }
            Event::LlmCalling => {}
            Event::Error { context, message, fatal } => {
                s.push(if *fatal { "💥" } else { "⚠️" },
                       format!("[{}] {}", context, message.chars().take(120).collect::<String>()),
                       if *fatal { "fail" } else { "info" });
            }
            Event::State { .. } => {}
            Event::BoxUp { compose_dir } => s.push("🐳", format!("box up: {}", compose_dir), "state"),
            Event::BoxDown { compose_dir } => s.push("🐳", format!("box down: {}", compose_dir), "state"),
            Event::SetupCommandStarted { service } => s.push("🔧", format!("setup running in '{}'", service), "info"),
            Event::SetupCommandFinished { service, success, excerpt } => {
                s.push(if *success { "🔧" } else { "⚠️" },
                       format!("setup {}: {}", service, excerpt.chars().take(100).collect::<String>()),
                       if *success { "info" } else { "fail" });
            }
            Event::BoxStarting { action, compose_dir } => {
                s.push("🐳", format!("→ {} — {}", action, compose_dir), "state");
            }
            Event::BoxOutput { stream, line, .. } => {
                let truncated: String = line.chars().take(300).collect();
                let icon = if stream == "stderr" { "⚠️" } else { "🐳" };
                s.push(icon, truncated, "box");
            }
            Event::BoxFinished { action, success, .. } => {
                let v = if *success { "✓" } else { "✗" };
                s.push("🐳", format!("{} {} finished", v, action),
                       if *success { "state" } else { "fail" });
            }
            Event::ServerUp { lab_id } => {
                s.server_up.insert(lab_id.clone(), true);
                s.push("🟢", format!("server up: {}", lab_id), "pass");
            }
            Event::ServerDown { lab_id, reason } => {
                s.server_up.insert(lab_id.clone(), false);
                s.push("🔴", format!("server down: {} — {}", lab_id, reason.chars().take(80).collect::<String>()), "fail");
            }
            Event::LabSkipped { lab_id, reason } => {
                s.push("⏭", format!("{} skipped: {}", lab_id, reason.chars().take(80).collect::<String>()), "fail");
            }
        }
    }
}

pub struct CompositeObserver {
    pub console: crate::events::ConsoleObserver,
    pub web: WebStateObserver,
}

impl EngineObserver for CompositeObserver {
    fn on_event(&self, event: Event) {
        self.console.on_event(event.clone());
        self.web.on_event(event);
    }
}