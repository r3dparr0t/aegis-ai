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
        if self.regex.is_match(input.body) {
            EvaluationResult {
                is_valid: true,
                error: None,
                error_details: None,
            }
        } else {
            EvaluationResult {
                is_valid: false,
                error: Some(EvaluationError::LogicFailure(
                    "regex_pattern_not_found".into(),
                )),
                error_details: Some(format!(
                    "Response did not match pattern '{}'. Got: {}",
                    self.source,
                    input.body.chars().take(300).collect::<String>()
                )),
            }
        }
    }
}