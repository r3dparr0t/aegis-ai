// src/provider/openai_compatible.rs
use std::time::Instant;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::domain::{LlmError, LlmProvider, LlmRequest, LlmResponse};

/// Provider عمومی برای هر API که از قرارداد رایج «OpenAI-compatible chat completions»
/// پیروی می‌کنه (OpenAI خودش، DeepSeek، Groq، Together، Fireworks، Mistral، و اکثر
/// APIهای جدیدتر که ادعای «OpenAI-compatible» دارن). برای اضافه‌کردن یک provider جدید
/// از این خانواده، کافیه یک نمونه‌ی جدید با base_url/api_key/model متفاوت بسازی —
/// نیازی به نوشتن کد جدید نیست، فقط این فایل رو استفاده کن.
///
/// ⚠️ فرض پیش‌فرض این پیاده‌سازی: بدنه‌ی درخواست/پاسخ دقیقاً همون schema رایج
/// OpenAI Chat Completions است (`POST {base_url}/chat/completions`،
/// `Authorization: Bearer <key>`، خروجی از `choices[0].message.content`).
/// اگر یک API خاص از این قرارداد پیروی نکنه (مثلاً یک API «محاسباتی» با ورودی/خروجی
/// متفاوت)، باید structهای Serialize/Deserialize پایین رو با مستندات واقعی آن API
/// تطبیق داد — ساختار کلی provider (trait، error handling، پارس latency) همون می‌مونه.
pub struct OpenAiCompatibleProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model_name: String,
}

impl OpenAiCompatibleProvider {
    /// api_key رو مستقیماً پاس بده. ترجیحاً به‌جای هاردکد کردن، از from_env استفاده کن.
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>, model_name: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            model_name: model_name.into(),
        }
    }

    /// api_key رو از یک متغیر محیطی (مثلاً "JEV_API_KEY") می‌خونه، نه از کد.
    /// اگر متغیر ست نشده باشه، یک پیام خطای واضح برمی‌گردونه به‌جای panic خاموش.
    pub fn from_env(
        base_url: impl Into<String>,
        api_key_env_var: &str,
        model_name: impl Into<String>,
    ) -> Result<Self, String> {
        let api_key = std::env::var(api_key_env_var)
            .map_err(|_| format!("Environment variable '{}' is not set", api_key_env_var))?;
        Ok(Self::new(base_url, api_key, model_name))
    }
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
    model: Option<String>,
    usage: Option<ChatUsage>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessageOwned,
}

#[derive(Deserialize)]
struct ChatMessageOwned {
    content: String,
}

#[derive(Deserialize)]
struct ChatUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleProvider {
    async fn generate(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError> {
        let endpoint = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));

        let payload = ChatCompletionRequest {
            model: &self.model_name,
            messages: vec![
                ChatMessage { role: "system", content: &request.system_prompt },
                ChatMessage { role: "user", content: &request.user_input },
            ],
            temperature: request.temperature,
            max_tokens: request.max_tokens,
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
            let body_snippet = http_response.text().await.unwrap_or_default();
            return Err(if status.as_u16() == 429 {
                LlmError::RateLimited
            } else {
                LlmError::ProviderUnavailable(format!(
                    "API returned {}: {}",
                    status,
                    body_snippet.chars().take(300).collect::<String>()
                ))
            });
        }

        let parsed: ChatCompletionResponse = http_response
            .json()
            .await
            .map_err(|e| LlmError::InvalidResponse(e.to_string()))?;

        let latency_ms = start_time.elapsed().as_millis() as u64;

        let output = parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| LlmError::InvalidResponse("no choices returned in response".to_string()))?;

        Ok(LlmResponse {
            output,
            model: parsed.model.or_else(|| Some(self.model_name.clone())),
            prompt_tokens: parsed.usage.as_ref().and_then(|u| u.prompt_tokens),
            completion_tokens: parsed.usage.as_ref().and_then(|u| u.completion_tokens),
            latency_ms,
        })
    }
}
