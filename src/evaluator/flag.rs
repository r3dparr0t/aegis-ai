// src/evaluator/flag.rs
use crate::domain::{EvaluationError, EvaluationInput, EvaluationResult, Evaluator};

/// ارزیابی موفقیت یک payload بر اساس *پاسخ واقعی هدف* (نه خروجی خام مدل).
/// برای سناریوهایی مثل SSRF که موفقیت یعنی رسیدن به یک منبع داخلی و دیدن یک نشانه‌ی مشخص.
pub struct FlagEvaluator {
    success_marker: String,
}

impl FlagEvaluator {
    pub fn new(success_marker: impl Into<String>) -> Self {
        Self {
            success_marker: success_marker.into(),
        }
    }
}

impl Evaluator for FlagEvaluator {
    fn evaluate(&self, input: EvaluationInput<'_>) -> EvaluationResult {
        if input.body.contains(&self.success_marker) {
            return EvaluationResult {
                is_valid: true,
                error: None,
                error_details: None,
            };
        }
        let category = match input.status_code {
                403 => EvaluationError::Custom("egress_blocked".into()),
                500 if input.body.contains("Connection refused") => EvaluationError::Custom("connection_refused".into()),
                500 if input.body.contains("Name or service not known") => EvaluationError::Custom("dns_resolution_failed".into()),
                _ => EvaluationError::LogicFailure("target response did not contain the success marker".into()),
            };
            EvaluationResult {
                is_valid: false,
                error: Some(category),
                error_details: Some(format!("HTTP {} — {}", input.status_code,
                    input.body.chars().take(300).collect::<String>())),
            }
    }
}
