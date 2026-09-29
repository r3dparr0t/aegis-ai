// src/labs/lab.rs
use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::{
    domain::Evaluator,
    engine::EngineConfig,
    evaluator::{FlagEvaluator, TimeDelayEvaluator},
    executor::HttpTargetExecutor,
};

use super::spec::{EvaluatorSpec, InternalTargetSpec, LabSpec};

/// قفل global تک‌نفره: فقط یک Lab در کل برنامه (چه از CLI چه از وب) می‌تونه
/// هم‌زمان در حال اجرا باشه. مقدارش، اگه Some باشه، همون Labیه که الان داره
/// اجرا می‌شه.
///
/// چرا global و نه per-lab؟ چون خودِ Engine حین اجرا رخدادهایی مثل
/// `AttemptStarted` / `PayloadParsed` / ... صادر می‌کنه که هیچ‌کدوم `lab_id`
/// ندارن (فقط `LabStarted`/`LabFinished` دارن). اگه دو تا Lab متفاوت هم‌زمان
/// اجرا بشن، این رخدادهای میانی معلوم نیست مال کدوم لبن — لاگ و progress قاطی
/// می‌شه. پس تا وقتی event stream این شکلیه، باید فقط یک اجرای فعال در کل
/// برنامه وجود داشته باشه؛ قفل per-lab (`Lab` خودش) این رو تضمین نمی‌کنه.
pub type RunLock = Arc<Mutex<Option<Arc<Lab>>>>;

/// وضعیت runtime یک Lab. با اجرا/اتمام تغییر می‌کنه.
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

/// یک Lab = spec (immutable، از YAML) + state (runtime، mutable) + متدهایی
/// که از روی spec، اجزای اجرا (executor، evaluator، config، prompt) رو می‌سازن.
pub struct Lab {
    pub spec: LabSpec,
    state: Mutex<LabState>,
}

impl Lab {
    pub fn new(spec: LabSpec) -> Self {
        Self {
            spec,
            state: Mutex::new(LabState::Untouched),
        }
    }

    // ─── Accessors ───

    pub fn id(&self) -> &str {
        &self.spec.meta.id
    }

    pub fn name(&self) -> &str {
        &self.spec.meta.name
    }

    pub fn description(&self) -> &str {
        &self.spec.meta.description
    }

    pub fn url(&self) -> &str {
        &self.spec.target.url
    }

    pub fn internal_url(&self) -> String {
        self.spec.internal_target.url()
    }

    pub fn max_attempts(&self) -> u32 {
        self.spec.max_attempts
    }

    // ─── State ───

    pub fn state(&self) -> LabState {
        *self.state.lock().unwrap()
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state(), LabState::Running { .. })
    }

    /// اگه *هیچ* Labی (نه فقط همین یکی) در حال اجرا نباشه، `run_lock` global رو
    /// می‌گیره و `true` برمی‌گردونه. وگرنه `false` — caller باید صرف‌نظر کنه.
    ///
    /// باید `Arc<Lab>` بگیره (نه `&self`) چون لازمه یه اشاره‌ی مالکیت‌دار به
    /// همین Lab رو داخل قفل ذخیره کنه تا `finish` بعداً بتونه مطمئن بشه داره
    /// قفلِ *خودش* رو آزاد می‌کنه، نه قفلی که یه Lab دیگه بین این دو تا گرفته.
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

    /// آپدیت شماره‌ی attempt جاری — فقط وقتی واقعاً Running باشه اثر می‌کنه.
    pub fn set_progress(&self, attempt: u32, max: u32) {
        let mut s = self.state.lock().unwrap();
        if matches!(*s, LabState::Running { .. }) {
            *s = LabState::Running { attempt, max };
        }
    }

    /// پایان اجرا. `success = true` → Passed، وگرنه Failed. قفل global رو هم
    /// آزاد می‌کنه — ولی فقط اگه هنوز مال همین Lab باشه (نه یه اجرای بعدی).
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

    /// برگرداندن به حالت اولیه (وقتی کاربر بخواد وضعیت رو دستی ریست کنه).
    /// این کاری با `run_lock` نداره — فقط برای Labهایی که در حال اجرا نیستن.
    pub fn reset(&self) {
        *self.state.lock().unwrap() = LabState::Untouched;
    }

    // ─── Runtime construction ───

    /// task_type نهایی — اگه `fresh_task_type` باشه، یه پسوند یکتا اضافه می‌شه.
    pub fn effective_task_type(&self) -> String {
        let base = &self.spec.task.task_type;
        if self.spec.task.fresh_task_type {
            let suffix = &uuid::Uuid::new_v4().simple().to_string()[..8];
            format!("{}_{}", base, suffix)
        } else {
            base.clone()
        }
    }

    /// executor برای این lab.
    pub fn build_executor(&self) -> Result<Arc<HttpTargetExecutor>, String> {
        let (base, path) = self
            .spec
            .target
            .split_base_path()
            .ok_or_else(|| format!("invalid target.url: {}", self.spec.target.url))?;
        Ok(Arc::new(HttpTargetExecutor::new(base, vec![path.as_str()])))
    }

    /// evaluator برای این lab (بر اساس `spec.evaluator`).
    pub fn build_evaluator(&self) -> Arc<dyn Evaluator> {
        match &self.spec.evaluator {
            EvaluatorSpec::Flag { marker } => Arc::new(FlagEvaluator::new(marker.clone())),
            EvaluatorSpec::TimeDelay { threshold_ms } => {
                Arc::new(TimeDelayEvaluator::new(*threshold_ms))
            }
        }
    }

    /// EngineConfig مناسب این lab.
    pub fn build_config(&self, task_type: String) -> EngineConfig {
        EngineConfig {
            max_attempts: self.spec.max_attempts,
            task_type,
            expected_body_key: if self.spec.target.body_key.is_empty() {
                None
            } else {
                Some(self.spec.target.body_key.clone())
            },
        }
    }

    /// system prompt نهایی — prefix (از context) + placeholderهای پر‌شده.
    pub fn render_system_prompt(&self, prefix: &str) -> String {
        let filled = fill(&self.spec.system_prompt, &self.spec.internal_target);
        format!("{}\n\n{}", prefix, filled)
    }

    /// goal پیش‌فرض lab (با placeholderهای پر‌شده).
    pub fn default_goal(&self) -> String {
        fill(&self.spec.task.default_goal, &self.spec.internal_target)
    }
}

fn fill(text: &str, target: &InternalTargetSpec) -> String {
    text.replace("{{target_url}}", &target.url())
        .replace("{{target_host}}", &target.host)
        .replace("{{target_port}}", &target.port.to_string())
        .replace("{{target_path}}", &target.path)
}
