// src/provider/typesafe.rs
//
// ⚠️ توجه: TypeSafe یک API «ارزیابی/طبقه‌بندی» است، نه «تولید متن».
// endpoint آن POST /v1/systemone است و خروجی آن فقط choice / score / noul
// برمی‌گرداند — یعنی هیچ‌وقت متن آزاد (مثل JSON ایجنت SSRF) تولید نمی‌کند.
// این provider صرفاً برای این است که TypeSafe به‌عنوان یک ابزار کمکی
// (classifier/evaluator) در دسترس باشه، نه جایگزین LLM چت.

use std::time::Instant;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use crate::domain::{LlmError, LlmProvider, LlmRequest, LlmResponse};

pub struct TypesafeProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model_name: String,
}

impl TypesafeProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model_name: impl Into<String>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            model_name: model_name.into(),
        }
    }

    /// api_key را از یک متغیر محیطی می‌خواند. کلید را از
    /// https://console.typesafe.ai/keys بگیر و توی .env بذار:
    ///   TYPESAFE_API_KEY=tsk_xxx
    pub fn from_env(
        base_url: impl Into<String>,
        api_key_env_var: &str,
        model_name: impl Into<String>,
    ) -> Result<Self, String> {
        let api_key = std::env::var(api_key_env_var).map_err(|_| {
            format!(
                "Environment variable '{}' is not set. Get a key at https://console.typesafe.ai/keys and put it in .env",
                api_key_env_var
            )
        })?;
        Ok(Self::new(base_url, api_key, model_name))
    }
}

#[derive(Serialize)]
struct SystemOneRequest<'a> {
    state: &'a str,
    model: &'a str,
    questions: serde_json::Value,
}

#[derive(Deserialize)]
struct SystemOneResponse {
    model: Option<String>,
    answers: serde_json::Value,
    usage: Option<SystemOneUsage>,
}

#[derive(Deserialize)]
struct SystemOneUsage {
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
}

#[async_trait]
impl LlmProvider for TypesafeProvider {
    async fn generate(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError> {
        let endpoint = format!("{}/systemone", self.base_url.trim_end_matches('/'));

        // system_prompt و user_input را در یک متن واحد (state) ترکیب می‌کنیم
        let state = format!(
            "{}\n\n---\n\n{}",
            request.system_prompt, request.user_input
        );

        // ⚠️ اینجا فقط یک سؤال noul می‌پرسیم چون API فقط choice/score/noul دارد.
        // اگه می‌خوای خروجی ساختاریافته‌ی خاصی بگیری، باید questions را
        // مطابق سناریو داینامیک بسازی (مثلاً choice با criteria مناسب).
        let questions = json!({
            "response_is_plausible": {
                "type": "noul",
                "instructions": "Is the assistant's proposed next step valid and safe to execute?"
            }
        });

        let payload = SystemOneRequest {
            state: &state,
            model: &self.model_name,
            questions,
        };

        let start_time = Instant::now();

        let http_response = self
            .client
            .post(&endpoint)
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    LlmError::Timeout
                } else {
                    LlmError::ProviderUnavailable(e.to_string())
                }
            })?;

        if !http_response.status().is_success() {
            let status = http_response.status();
            let body = http_response.text().await.unwrap_or_default();
            return Err(if status.as_u16() == 429 {
                LlmError::RateLimited
            } else {
                LlmError::ProviderUnavailable(format!(
                    "TypeSafe returned {}: {}",
                    status,
                    body.chars().take(300).collect::<String>()
                ))
            });
        }

        let parsed: SystemOneResponse = http_response
            .json()
            .await
            .map_err(|e| LlmError::InvalidResponse(e.to_string()))?;

        let latency_ms = start_time.elapsed().as_millis() as u64;

        // خروجی TypeSafe (که یک object ساختاریافته است) را به‌صورت
        // JSON string برمی‌گردانیم تا trait LlmProvider راضی بشه.
        // ⚠️ این خروجی، JSON مورد انتظار ایجنت SSRF نیست؛ صرفاً خروجی خام TypeSafe است.
        let output = serde_json::to_string_pretty(&parsed.answers)
            .unwrap_or_else(|_| "{}".to_string());

        Ok(LlmResponse {
            output,
            model: parsed.model.or_else(|| Some(self.model_name.clone())),
            prompt_tokens: parsed.usage.as_ref().and_then(|u| u.input_tokens),
            completion_tokens: parsed.usage.as_ref().and_then(|u| u.output_tokens),
            latency_ms,
        })
    }
}