// src/domain/llm.rs
use async_trait::async_trait;
use crate::domain::error::LlmError;

/// درخواست ورودی به مدل
#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub system_prompt: String,
    pub user_input: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

/// خروجی مدل - متادیتای توکن و مدل جهت انعطاف با Providerهای مختلف Option شدند
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub output: String,
    pub model: Option<String>,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub latency_ms: u64,
}

/// اینترفیس تعامل با تامین‌کنندگان LLM
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn generate(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError>;
}
