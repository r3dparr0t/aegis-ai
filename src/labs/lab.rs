// src/labs/lab.rs
use std::sync::{Arc, Mutex, RwLock};

use serde::Serialize;

use crate::{
    domain::Evaluator,
    engine::EngineConfig,
    evaluator::{FlagEvaluator, TimeDelayEvaluator},
    executor::HttpTargetExecutor,
};

use super::spec::{EvaluatorSpec, InternalTargetSpec, LabOptions, LabSpec};

/// قفل global تک‌نفره: فقط یک Lab در کل برنامه می‌تونه هم‌زمان در حال اجرا باشه.
/// (نگاه کن به توضیح مفصل‌تر در نسخه‌ی قبلی — رخدادهای بین‌راهی lab_id ندارن،
/// پس قفل باید global باشه نه per-lab.)
pub type RunLock = Arc<Mutex<Option<Arc<Lab>>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LabState {
    Untouched,
    Running { attempt: u32, max: u32 },
    Passed { attempts: u32 },
    Failed { attempts: u32 },
}

impl LabState {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Untouched => "untouched",
            Self::Running { .. } => "running",
            Self::Passed { .. } => "passed",
            Self::Failed { .. } => "failed",
        }
    }
}

/// یک Lab = spec (پشت RwLock، چون از پنل وب قابل ادیت/جایگزینیه) + state (runtime).
pub struct Lab {
    spec: RwLock<LabSpec>,
    state: Mutex<LabState>,
}

impl Lab {
    pub fn new(spec: LabSpec) -> Self {
        Self {
            spec: RwLock::new(spec),
            state: Mutex::new(LabState::Untouched),
        }
    }

    // ─── Spec (snapshot-based، چون قابل تعویضه) ───

    pub fn spec(&self) -> LabSpec {
        self.spec.read().unwrap().clone()
    }

    pub fn id(&self) -> String {
        self.spec.read().unwrap().meta.id.clone()
    }

    pub fn name(&self) -> String {
        self.spec.read().unwrap().meta.name.clone()
    }

    pub fn description(&self) -> String {
        self.spec.read().unwrap().meta.description.clone()
    }

    pub fn url(&self) -> String {
        self.spec.read().unwrap().target.url.clone()
    }

    pub fn internal_url(&self) -> String {
        self.spec.read().unwrap().internal_target.url()
    }

    pub fn max_attempts(&self) -> u32 {
        self.spec.read().unwrap().max_attempts
    }

    pub fn options(&self) -> LabOptions {
        self.spec.read().unwrap().options.clone()
    }

    /// جایگزین‌کردن spec با نسخه‌ی جدید (از پنل وب، بعد از ادیت YAML).
    /// - اگه در حال اجرا باشه → رد (یه TOCTOU خفیف اینجا هست: بین این چک و
    ///   نوشتن واقعی، تئوریاً می‌شه یه اجرا از CLI شروع بشه؛ برای این اپ که
    ///   ادیت‌ها دستی و کم‌تعدادن، اهمیتی نداره — ولی اگه لازم شد، باید
    ///   `is_running` و نوشتن رو زیر یک قفل مشترک با `try_start` برد)
    /// - اگه id عوض شده باشه → رد (وگرنه فایل YAML روی دیسک گم می‌شه)
    pub fn replace_spec(&self, new_spec: LabSpec) -> Result<(), String> {
        if self.is_running() {
            return Err("cannot edit a running lab".into());
        }
        let old_id = self.spec.read().unwrap().meta.id.clone();
        if new_spec.meta.id != old_id {
            return Err(format!(
                "lab id cannot change on save: '{}' → '{}'",
                old_id, new_spec.meta.id
            ));
        }
        *self.spec.write().unwrap() = new_spec;
        Ok(())
    }

    // ─── State ───

    pub fn state(&self) -> LabState {
        *self.state.lock().unwrap()
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state(), LabState::Running { .. })
    }

    pub fn try_start(self_arc: &Arc<Lab>, run_lock: &RunLock) -> bool {
        let mut guard = run_lock.lock().unwrap();
        if guard.is_some() {
            return false;
        }
        *self_arc.state.lock().unwrap() = LabState::Running {
            attempt: 0,
            max: self_arc.max_attempts(),
        };
        *guard = Some(self_arc.clone());
        true
    }

    pub fn set_progress(&self, attempt: u32, max: u32) {
        let mut s = self.state.lock().unwrap();
        if matches!(*s, LabState::Running { .. }) {
            *s = LabState::Running { attempt, max };
        }
    }

    pub fn finish(self_arc: &Arc<Lab>, run_lock: &RunLock, success: bool, attempts: u32) {
        *self_arc.state.lock().unwrap() = if success {
            LabState::Passed { attempts }
        } else {
            LabState::Failed { attempts }
        };

        let mut guard = run_lock.lock().unwrap();
        if let Some(current) = guard.as_ref() {
            if Arc::ptr_eq(current, self_arc) {
                *guard = None;
            }
        }
    }

    pub fn reset(&self) {
        *self.state.lock().unwrap() = LabState::Untouched;
    }

    // ─── Runtime construction ───

    pub fn effective_task_type(&self) -> String {
        let spec = self.spec.read().unwrap();
        let base = &spec.task.task_type;
        if spec.task.fresh_task_type {
            let suffix = &uuid::Uuid::new_v4().simple().to_string()[..8];
            format!("{}_{}", base, suffix)
        } else {
            base.clone()
        }
    }

    pub fn build_executor(&self) -> Result<Arc<HttpTargetExecutor>, String> {
        let spec = self.spec.read().unwrap();
        let (base, path) = spec
            .target
            .split_base_path()
            .ok_or_else(|| format!("invalid target.url: {}", spec.target.url))?;
        Ok(Arc::new(HttpTargetExecutor::new(base, vec![path.as_str()])))
    }

    pub fn build_evaluator(&self) -> Arc<dyn Evaluator> {
        let spec = self.spec.read().unwrap();
        match &spec.evaluator {
            EvaluatorSpec::Flag { marker } => Arc::new(FlagEvaluator::new(marker.clone())),
            EvaluatorSpec::TimeDelay { threshold_ms } => {
                Arc::new(TimeDelayEvaluator::new(*threshold_ms))
            }
        }
    }

    pub fn build_config(&self, task_type: String) -> EngineConfig {
        let spec = self.spec.read().unwrap();
        EngineConfig {
            max_attempts: spec.max_attempts,
            task_type,
            expected_body_key: if spec.target.body_key.is_empty() {
                None
            } else {
                Some(spec.target.body_key.clone())
            },
        }
    }

    pub fn render_system_prompt(&self, prefix: &str) -> String {
        let spec = self.spec.read().unwrap();
        let filled = fill(&spec.system_prompt, &spec.internal_target);
        format!("{}\n\n{}", prefix, filled)
    }

    pub fn default_goal(&self) -> String {
        let spec = self.spec.read().unwrap();
        fill(&spec.task.default_goal, &spec.internal_target)
    }
}

fn fill(text: &str, target: &InternalTargetSpec) -> String {
    text.replace("{{target_url}}", &target.url())
        .replace("{{target_host}}", &target.host)
        .replace("{{target_port}}", &target.port.to_string())
        .replace("{{target_path}}", &target.path)
}
