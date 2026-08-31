// src/evaluator/json.rs
use serde_json::Value;
use crate::domain::{EvaluationError, EvaluationResult, Evaluator};

/// ارزیاب خروجی‌های ساختاریافته JSON
pub struct JsonEvaluator {
    required_keys: Vec<String>,
}

impl JsonEvaluator {
    /// تعریف ارزیاب جدید با لیستی از کلیدهای الزامی در سند JSON
    pub fn new(required_keys: Vec<&str>) -> Self {
        Self {
            required_keys: required_keys.into_iter().map(|s| s.to_string()).collect(),
        }
    }

    /// تمیزکاری اولیه متن خروجی (استخراج JSON از میان Fencesهای مارک‌داون مثل ```json)
    fn clean_output(output: &str) -> String {
        let trimmed = output.trim();
        if trimmed.starts_with("```json") {
            trimmed
                .strip_prefix("```json")
                .unwrap_or(trimmed)
                .strip_suffix("```")
                .unwrap_or(trimmed)
                .trim()
                .to_string()
        } else if trimmed.starts_with("```") {
            trimmed
                .strip_prefix("```")
                .unwrap_or(trimmed)
                .strip_suffix("```")
                .unwrap_or(trimmed)
                .trim()
                .to_string()
        } else {
            trimmed.to_string()
        }
    }
}

impl Evaluator for JsonEvaluator {
    fn evaluate(&self, output: &str) -> EvaluationResult {
        let cleaned = Self::clean_output(output);

        // ۱. بررسی پارس اولیه JSON
        let json_value: Value = match serde_json::from_str(&cleaned) {
            Ok(v) => v,
            Err(err) => {
                return EvaluationResult {
                    is_valid: false,
                    error: Some(EvaluationError::InvalidJson),
                    error_details: Some(format!("Failed to parse JSON: {}", err)),
                };
            }
        };

        // ۲. مطمئن شدن از اینکه خروجی یک JSON Object است (نه Array یا Primitive)
        let obj = match json_value.as_object() {
            Some(obj) => obj,
            None => {
                return EvaluationResult {
                    is_valid: false,
                    error: Some(EvaluationError::SchemaMismatch),
                    error_details: Some("Expected JSON Object at root level, got array or primitive".to_string()),
                };
            }
        };

        // ۳. بررسی وجود کلیدهای الزامی (Required Fields)
        for key in &self.required_keys {
            if !obj.contains_key(key) {
                return EvaluationResult {
                    is_valid: false,
                    error: Some(EvaluationError::MissingField),
                    error_details: Some(format!("Missing required JSON key: '{}'", key)),
                };
            }
        }

        // خروجی کاملاً معتبر است
        EvaluationResult {
            is_valid: true,
            error: None,
            error_details: None,
        }
    }
}
