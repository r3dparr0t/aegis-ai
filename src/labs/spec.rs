// src/labs/spec.rs
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// آدرس داخلی (internal-admin) — فقط برای SSRF labها لازمه.
/// برای CVEهای غیر-SSRF می‌تونه غایب باشه.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InternalTargetSpec {
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl InternalTargetSpec {
    pub fn url(&self) -> String {
        format!("http://{}:{}{}", self.host, self.port, self.path)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LabSpec {
    pub meta: LabMeta,
    pub max_attempts: u32,
    pub target: TargetSpec,
    /// اختیاری — فقط اگه `system_prompt` از `{{target_url}}` استفاده کنه لازمه.
    #[serde(default)]
    pub internal_target: Option<InternalTargetSpec>,
    pub task: TaskSpec,
    pub evaluator: EvaluatorSpec,
    pub system_prompt: String,
    #[serde(default)]
    pub options: LabOptions,
    /// راهنمایی‌های پله‌ای — بعد از N تلاش ناموفق، متن کمکی به system_prompt
    /// اضافه می‌شه. نگاه کن به `HintSpec` برای معنای دقیق `after_attempt`.
    #[serde(default)]
    pub hints: Vec<HintSpec>,
    /// اگه ست بشه، دقیقاً سر یه attempt مشخص، مدل این *اجرا* (نه global)
    /// موقتاً به یه مدل دیگه سوییچ می‌کنه — فقط وقتی provider فعلی Ollama
    /// باشه. provider عمومی/global دست‌نخورده می‌مونه.
    #[serde(default)]
    pub fallback_strategy: Option<FallbackSpec>,
     #[serde(default)]
    pub vulhub: Option<VulhubSpec>,
}

/// `after_attempt: 3` یعنی: *بعد از* شکست‌خوردنِ کامل تلاش شماره‌ی ۳ (یعنی
/// شروع از تلاش ۴ به بعد)، این متن به system_prompt اضافه می‌شه. تا وقتی لب
/// جواب بده یا تلاش‌ها تموم بشه، هر hintی که فعال شده باشه، تو همه‌ی
/// تلاش‌های بعدی باقی می‌مونه (مثل lessons).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HintSpec {
    pub after_attempt: u32,
    pub message: String,
}

/// `on_attempt: 4` یعنی: دقیقاً سرِ شروع تلاش شماره‌ی ۴ (نه قبل، نه بعد)،
/// provider این یه اجرا موقتاً عوض می‌شه به `target_model` — فقط اگه
/// provider فعلی Ollama باشه (برای provider غیر-Ollama نادیده گرفته می‌شه،
/// چون سوییچ زنده‌ی مدل فقط برای Ollama معنی داره).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FallbackSpec {
    pub on_attempt: u32,
    pub target_model: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
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

fn default_true() -> bool {
    true
}

// ═══════════════════════════════════════════════════════════════
// Target: دو حالت — ssrf_json یا raw
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TargetSpec {
    /// SSRF lab: مدل `{"endpoint": "...", "body": {...}}` می‌ده،
    /// executor یه POST JSON به `{url}{endpoint}` می‌زنه.
    SsrfJson {
        /// base URL اپ آسیب‌پذیر (مثلاً http://localhost:5000)
        url: String,
        /// کلید body که مدل باید بذاره (مثلاً "url" یا "target_url")
        #[serde(default)]
        body_key: String,
    },
    /// Raw HTTP: مدل `{"method": "...", "path": "...", "body": "..."}` می‌ده،
    /// executor عیناً همون رو می‌فرسته. برای CVEها.
    Raw {
        /// base URL (مثلاً http://localhost:8080) — path از payload اضافه می‌شه
        url: String,
        /// متد پیش‌فرض اگه مدل نداده باشه
        #[serde(default = "default_method")]
        method: String,
        /// هدرهای ثابت که به همه‌ی درخواست‌ها اضافه می‌شن
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}

fn default_method() -> String {
    "POST".to_string()
}

impl TargetSpec {
    pub fn base_url(&self) -> &str {
        match self {
            TargetSpec::SsrfJson { url, .. } => url,
            TargetSpec::Raw { url, .. } => url,
        }
    }

    /// برای ssrf_json: `(base, path)` جدا می‌شه چون executor فقط یه endpoint
    /// محدود قبول می‌کنه. برای raw: هیچ محدودیتی نیست.
    pub fn split_base_path(&self) -> Option<(String, String)> {
        match self {
            TargetSpec::SsrfJson { url, .. } => {
                let parsed = url::Url::parse(url).ok()?;
                let base = format!("{}://{}", parsed.scheme(), parsed.authority());
                let path = parsed.path().to_string();
                Some((base, path))
            }
            TargetSpec::Raw { url, .. } => Some((url.clone(), String::new())),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TaskSpec {
    pub task_type: String,
    pub default_goal: String,
    #[serde(default)]
    pub fresh_task_type: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvaluatorSpec {
    Flag { marker: String },
    TimeDelay { threshold_ms: u64 },
    Regex { pattern: String },
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct LabOptions {
    #[serde(default)]
    pub allow_bigger_model: bool,
    #[serde(default)]
    pub default_bigger_model: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VulhubSpec {
    /// مسیر پوشه‌ی حاوی docker-compose.yml. اگه نسبی باشه، نسبت به
    /// `paths.vulhub_root` تو aegis.toml تفسیر می‌شه.
    pub compose_dir: String,
    /// بعد از `docker compose up -d` چند ثانیه صبر کنیم (پیش‌فرض ۳).
    #[serde(default = "default_wait_secs")]
    pub wait_secs: u64,
}

fn default_wait_secs() -> u64 { 3 }