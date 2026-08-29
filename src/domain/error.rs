// src/domain/error.rs
use std::fmt;
use crate::domain::execution::EvaluationError;

/// خطاهای لایه ارتباطی و زیرساختی LLM
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmError {
    Timeout,
    RateLimited,
    Authentication,
    ProviderUnavailable(String),
    InvalidResponse(String),
    ContextWindowExceeded,
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmError::Timeout => write!(f, "Request timed out"),
            LlmError::RateLimited => write!(f, "Rate limit exceeded"),
            LlmError::Authentication => write!(f, "Authentication failed"),
            LlmError::ProviderUnavailable(msg) => write!(f, "Provider unavailable: {}", msg),
            LlmError::InvalidResponse(msg) => write!(f, "Invalid response format: {}", msg),
            LlmError::ContextWindowExceeded => write!(f, "Context window limit reached"),
        }
    }
}

impl std::error::Error for LlmError {}

impl LlmError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, LlmError::Timeout | LlmError::RateLimited | LlmError::ProviderUnavailable(_))
    }
}

/// خطاهای کل سیستم ارکستراتور
#[derive(Debug)]
pub enum EngineError {
    Llm(LlmError),
    Evaluation(EvaluationError),
    Database(String),
    MaxAttemptsExceeded { execution_id: String, attempts: u32 },
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::Llm(err) => write!(f, "LLM Transport Error: {}", err),
            EngineError::Evaluation(err) => write!(f, "Evaluation Failure: {}", err),
            EngineError::Database(err) => write!(f, "Database Error: {}", err),
            EngineError::MaxAttemptsExceeded { execution_id, attempts } => {
                write!(f, "Execution {} failed after {} attempts", execution_id, attempts)
            }
        }
    }
}

impl std::error::Error for EngineError {}
