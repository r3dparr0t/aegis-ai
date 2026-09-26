// src/domain/execution.rs
use std::fmt;

/// خطاهایی که ارزیاب (Evaluator) در خروجی مدل کشف می‌کند
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvaluationError {
    InvalidJson,
    SchemaMismatch,
    MissingField,
    InvalidFormat,
    LogicFailure(String),
    Custom(String),
}

impl fmt::Display for EvaluationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvaluationError::InvalidJson => write!(f, "invalid_json"),
            EvaluationError::SchemaMismatch => write!(f, "schema_mismatch"),
            EvaluationError::MissingField => write!(f, "missing_field"),
            EvaluationError::InvalidFormat => write!(f, "invalid_format"),
            EvaluationError::LogicFailure(msg) => write!(f, "logic_failure: {}", msg),
            EvaluationError::Custom(val) => write!(f, "{}", val),
        }
    }
}

/// نتیجه ارزیابی خروجی مدل
#[derive(Debug, Clone)]
pub struct EvaluationResult {
    pub is_valid: bool,
    pub error: Option<EvaluationError>,
    pub error_details: Option<String>,
}

/// ورودی استاندارد ارزیابی — نه فقط body، بلکه status و latency هم
/// (لازم برای Evaluatorهایی مثل TimeDelayEvaluator که body خالیه و فقط latency مهمه)
pub struct EvaluationInput<'a> {
    pub body: &'a str,
    pub status_code: u16,
    pub latency_ms: u64,
}

/// اینترفیس ارزیاب مستقل
pub trait Evaluator: Send + Sync {
    fn evaluate(&self, input: EvaluationInput<'_>) -> EvaluationResult;
}