// src/evaluator/regex.rs
use regex::Regex;

use crate::domain::{Evaluator, EvaluationError, EvaluationInput, EvaluationResult};

/// ارزیاب مبتنی بر regex: موفق = پاسخ هدف با الگو مطابقت داره.
/// برای CVEها استفاده می‌شه (مثلاً دنبال `root:.*:0:0:` تو /etc/passwd).
pub struct RegexEvaluator {
    regex: Regex,
    source: String,
}

impl RegexEvaluator {
    pub fn new(pattern: &str) -> Result<Self, String> {
        let regex = Regex::new(pattern).map_err(|e| format!("invalid regex: {}", e))?;
        Ok(Self {
            regex,
            source: pattern.to_string(),
        })
    }
}

impl Evaluator for RegexEvaluator {
    fn evaluate(&self, input: EvaluationInput<'_>) -> EvaluationResult {
       // اول body، بعد headers
        let mut haystack = input.body.to_string();
        for (_, v) in &input.headers {
            haystack.push('\n');
            haystack.push_str(v);
        }
         // ۱. اول بدنه
        if self.regex.is_match(&input.body) {
            return EvaluationResult {
                is_valid: true,
                error: None,
                error_details: Some("matched in body".to_string()),
            };
        }

        // ۲. بعد مقادیر هدرها (name فقط برای پیام گزارش)
        for (name, value) in &input.headers {
            if self.regex.is_match(value) {
                return EvaluationResult {
                    is_valid: true,
                    error: None,
                    error_details: Some(format!("matched in header '{}'", name)),
                };
            }
        }

        // ۳. شکست — هم بدنه هم هدرها را در error_details بگذار
        EvaluationResult {
            is_valid: false,
            error: Some(EvaluationError::LogicFailure(
                "regex_pattern_not_found".into(),
            )),
            error_details: Some(format!(
                "Response did not match pattern '{}'. Body (first 300): {} | Headers: {:?}",
                self.source,
                input.body.chars().take(300).collect::<String>(),
                input.headers
            )),
        }
    }
}
