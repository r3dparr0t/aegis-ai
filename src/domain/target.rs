// src/domain/target.rs
use std::fmt;
use async_trait::async_trait;
use serde_json::Value;

/// نتیجه‌ی واقعی اجرای یک payload روی هدف (نه خروجی خام مدل)
#[derive(Debug, Clone)]
pub struct ExecutionOutcome {
    pub status_code: u16,
    pub body: String,
    pub latency_ms: u64,
}

/// خطاهای مربوط به اجرای یک payload روی هدف واقعی
#[derive(Debug, Clone)]
pub enum ExecutorError {
    /// payload تولیدشده توسط مدل ساختار موردنیاز Executor را نداشت
    InvalidPayload(String),
    /// endpoint درخواستی در لیست مجاز نیست
    UnknownEndpoint(String),
    /// خطای شبکه‌ای هنگام درخواست به هدف
    Network(String),
}

impl fmt::Display for ExecutorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecutorError::InvalidPayload(msg) => write!(f, "invalid_payload: {}", msg),
            ExecutorError::UnknownEndpoint(ep) => write!(f, "unknown_endpoint: {}", ep),
            ExecutorError::Network(msg) => write!(f, "network_error: {}", msg),
        }
    }
}

impl std::error::Error for ExecutorError {}

/// اینترفیس اجرای واقعی payload تولیدشده توسط مدل بر روی یک هدف (مثلاً یک HTTP API آسیب‌پذیر)
#[async_trait]
pub trait TargetExecutor: Send + Sync {
    async fn execute(&self, payload: &Value) -> Result<ExecutionOutcome, ExecutorError>;
}
