use std::sync::{Arc, Mutex};
use serde::Serialize;
use crate::events::{EngineObserver, Event};

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LabStatus {
    Idle,
    Running { attempt: u32, max: u32 },
    Passed { attempts: u32 },
    Failed { attempts: u32 },
}

#[derive(Clone, Debug, Serialize)]
pub struct LogLine {
    pub time: u64,
    pub icon: String,
    pub text: String,
    pub kind: String,   // "info" / "pass" / "fail" / "state"
}

pub struct WebState {
    pub statuses: Vec<LabStatus>,
    pub log: Vec<LogLine>,
    pub active_lab: Option<usize>,
}

impl WebState {
    pub fn new(n: usize) -> Self {
        Self {
            statuses: vec![LabStatus::Idle; n],
            log: Vec::new(),
            active_lab: None,
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

pub type SharedWebState = Arc<Mutex<WebState>>;

/// Observer که eventها رو به WebState می‌ریزه.
pub struct WebStateObserver {
    pub state: SharedWebState,
}

impl EngineObserver for WebStateObserver {
    fn on_event(&self, event: Event) {
        let mut s = self.state.lock().unwrap();
        let active = s.active_lab;
        match &event {
            Event::LabStarted { lab_id, .. } => {
                // active_lab رو از main.rs ست می‌کنیم؛ اما اگه از web اجرا شد،
                // همین رو دوباره تأیید می‌کنیم.
                let _ = lab_id;
            }
            Event::AttemptStarted { attempt, max_attempts, .. } => {
                if let Some(i) = active {
                    s.statuses[i] = LabStatus::Running { attempt: *attempt, max: *max_attempts };
                }
                s.push("🔁", format!("Attempt {}/{}", attempt, max_attempts), "state");
            }
            Event::LessonsInjected { count, .. } => {
                if *count > 0 { s.push("🧠", format!("{} lesson(s)", count), "info"); }
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
                if let Some(i) = active {
                    s.statuses[i] = if *success {
                        LabStatus::Passed { attempts: *attempts }
                    } else {
                        LabStatus::Failed { attempts: *attempts }
                    };
                }
                s.active_lab = None;
            }
            Event::LlmCalling => {}
            Event::Error { context, message, fatal } => {
                s.push(
                    if *fatal { "💥" } else { "⚠️" },
                    format!("[{}] {}", context, message.chars().take(120).collect::<String>()),
                    if *fatal { "fail" } else { "info" },
                );
            }
            Event::State { .. } => {}
        }
    }
}

/// Observer ترکیبی: هم console، هم web.
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