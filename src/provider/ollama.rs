// src/provider/ollama.rs
use std::time::Instant;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::domain::{LlmProvider, LlmRequest, LlmResponse};

/// DTO برای ارسال درخواست به REST API در Ollama (/api/generate)
#[derive(Serialize)]
struct OllamaGenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    system: &'a str,
    stream: bool,
}

/// DTO برای دریافت پاسخ از Ollama
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
    async fn generate(&self, request: &LlmRequest) -> Result<LlmResponse, String> {
        let endpoint = format!("{}/api/generate", self.base_url.trim_end_matches('/'));

        let payload = OllamaGenerateRequest {
            model: &self.model_name,
            prompt: &request.user_prompt,
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
            .map_err(|e| format!("HTTP request to Ollama failed: {}", e))?;

        if !http_response.status().is_success() {
            return Err(format!(
                "Ollama returned error status: {}",
                http_response.status()
            ));
        }

        let ollama_res: OllamaGenerateResponse = http_response
            .json()
            .await
            .map_err(|e| format!("Failed to parse Ollama JSON response: {}", e))?;

        let latency_ms = start_time.elapsed().as_millis() as u64;

        Ok(LlmResponse {
            output: ollama_res.response,
            prompt_tokens: ollama_res.prompt_eval_count,
            completion_tokens: ollama_res.eval_count,
            latency_ms,
        })
    }
}
