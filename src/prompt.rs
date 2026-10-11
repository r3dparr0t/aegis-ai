// src/prompt.rs
//! بارگذاری پرامپت‌های سراسری از پوشه‌ی prompts/.

use crate::config::Config;

/// یه prompt prefix آماده برای inject کردن به ابتدای هر system_prompt.
/// فقط برای labهای ssrf_json معنی داره (نگاه کن به Lab::render_system_prompt).
pub struct PromptPrefix {
    pub text: String,
}

impl PromptPrefix {
    pub fn load(config: &Config, is_local: bool) -> Result<Self, std::io::Error> {
        let format_rules = std::fs::read_to_string(
            config.paths.prompts_dir.join("format_rules.txt")
        )?.trim().to_string();

        let text = if is_local {
            let cft = std::fs::read_to_string(
                config.paths.prompts_dir.join("cft_context.txt")
            )?.trim().to_string();
            format!("{}\n\n{}", cft, format_rules)
        } else {
            format_rules
        };

        Ok(Self { text })
    }

    /// برای مکان‌هایی که فقط یه &str می‌خوان (مثل LabContext.prefix).
    pub fn as_str(&self) -> &str {
        &self.text
    }
}