// src/labs/spec.rs
use serde::Deserialize;

/// آدرس واقعی سرویس داخلی (internal-admin) که این لب باید بهش برسه.
/// این تنها منبع حقیقت برای host/port/path است — system_prompt دیگه این‌ها رو
/// هاردکد نمی‌کنه، بلکه با placeholder بهشون اشاره می‌کنه (نگاه کن به runner.rs).
#[derive(Debug, Clone, Deserialize)]
pub struct InternalTargetSpec {
    /// هاست یا IP سرویس داخلی، مثلاً "internal-admin" یا "172.28.0.10"
    /// (Lab 4 عمداً IP خام می‌ذاره چون هدف خودِ لب، بای‌پس کردن فیلتر هاست‌نیمه)
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl InternalTargetSpec {
    pub fn url(&self) -> String {
        format!("http://{}:{}{}", self.host, self.port, self.path)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LabSpec {
    pub meta: LabMeta,
    pub target: TargetSpec,
    pub internal_target: InternalTargetSpec,
    pub task: TaskSpec,
    pub evaluator: EvaluatorSpec,
    pub system_prompt: String,
    #[serde(default)]
    pub options: LabOptions,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LabMeta {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub order: u32,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool { true }

#[derive(Debug, Clone, Deserialize)]
pub struct TargetSpec {
    pub endpoint: String,
    #[serde(default)]
    pub body_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskSpec {
    pub task_type: String,
    pub default_goal: String,
    #[serde(default)]
    pub fresh_task_type: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvaluatorSpec {
    Flag { marker: String },
    TimeDelay { threshold_ms: u64 },
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LabOptions {
    #[serde(default)]
    pub allow_bigger_model: bool,
    #[serde(default)]
    pub default_bigger_model: String,
}
