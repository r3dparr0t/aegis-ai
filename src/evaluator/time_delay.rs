// src/evaluator/time_delay.rs
use crate::domain::{EvaluationError, EvaluationInput, EvaluationResult, Evaluator};

/// برای Blind SSRF مبتنی بر تاخیر: هیچ محتوایی برنمی‌گرده، فقط زمان پاسخ اهمیت داره.
/// اگه latency از آستانه بیشتر باشه، فرض می‌کنیم هدف درخواست SSRF رو واقعاً اجرا کرده
/// و به یه منبع داخلی که کند پاسخ می‌ده رسیده.
pub struct TimeDelayEvaluator {
    pub threshold_ms: u64,
}

impl TimeDelayEvaluator {
    pub fn new(threshold_ms: u64) -> Self {
        Self { threshold_ms }
    }
}

impl Evaluator for TimeDelayEvaluator {
    fn evaluate(&self, input: EvaluationInput<'_>) -> EvaluationResult {
        if input.latency_ms >= self.threshold_ms {
            EvaluationResult {
                is_valid: true,
                error: None,
                error_details: None,
            }
        } else {
            EvaluationResult {
                is_valid: false,
                error: Some(EvaluationError::Custom("time_delay_insufficient".to_string())),
                error_details: Some(format!(
                    "Response latency {}ms was below threshold {}ms — \
                     the target either did not perform the SSRF, or reached a fast endpoint.",
                    input.latency_ms, self.threshold_ms
                )),
            }
        }
    }
}