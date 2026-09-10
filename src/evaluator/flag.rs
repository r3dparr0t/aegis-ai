// src/evaluator/flag.rs
use crate::domain::{EvaluationError, EvaluationResult, Evaluator};

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
    fn evaluate(&self, target_response_body: &str) -> EvaluationResult {
        if target_response_body.contains(&self.success_marker) {
            EvaluationResult {
                is_valid: true,
                error: None,
                error_details: None,
            }
        } else {
            EvaluationResult {
                is_valid: false,
                error: Some(EvaluationError::LogicFailure(
                    "target response did not contain the success marker".to_string(),
                )),
                error_details: Some(format!(
                    "Expected to find '{}' in target response, got: {}",
                    self.success_marker,
                    target_response_body.chars().take(300).collect::<String>()
                )),
            }
        }
    }
}
