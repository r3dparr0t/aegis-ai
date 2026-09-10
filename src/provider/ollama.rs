// src/provider/ollama.rs
use std::time::Instant;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::domain::{LlmError, LlmProvider, LlmRequest, LlmResponse};

/// DTO برای ارسال درخواست به REST API در Ollama (/api/generate)
#[derive(Serialize)]
struct OllamaGenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    system: &'a str,
    stream: bool,
}

/// DTO برای دریافت پاسخ از Ollama
#[allow(dead_code)]
#[derive(Deserialize)]
struct OllamaGenerateResponse {
    response: String,
    done: bool,
    prompt_eval_count: Option<u32>,
    eval_count: Option<u32>,
}

/// پیاده‌سازی ارائه دهنده Ollama
pub struct OllamaProvider {
    client: reqwest::Client,
    base_url: String,
    model_name: String,
}

impl OllamaProvider {
    pub fn new(base_url: impl Into<String>, model_name: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            model_name: model_name.into(),
        }
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    async fn generate(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError> {
        let endpoint = format!("{}/api/generate", self.base_url.trim_end_matches('/'));

        let payload = OllamaGenerateRequest {
            model: &self.model_name,
            prompt: &request.user_input,
            system: &request.system_prompt,
            stream: false,
        };

        let start_time = Instant::now();

        let http_response = self
            .client
            .post(&endpoint)
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
            return Err(if status.as_u16() == 429 {
                LlmError::RateLimited
            } else {
                LlmError::ProviderUnavailable(format!("Ollama returned status: {}", status))
            });
        }

        let ollama_res: OllamaGenerateResponse = http_response
            .json()
            .await
            .map_err(|e| LlmError::InvalidResponse(e.to_string()))?;

        let latency_ms = start_time.elapsed().as_millis() as u64;

        Ok(LlmResponse {
            output: ollama_res.response,
            model: Some(self.model_name.clone()),
            prompt_tokens: ollama_res.prompt_eval_count,
            completion_tokens: ollama_res.eval_count,
            latency_ms,
        })
    }
}
