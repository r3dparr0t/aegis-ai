// src/evaluator/regex.rs
use regex::Regex;
use crate::domain::{EvaluationError, EvaluationInput, EvaluationResult, Evaluator};

pub struct RegexEvaluator {
    pattern: Regex,
    expected_description: String,
}

impl RegexEvaluator {
    pub fn new(pattern_str: &str, description: &str) -> Result<Self, regex::Error> {
        let pattern = Regex::new(pattern_str)?;
        Ok(Self {
            pattern,
            expected_description: description.to_string(),
        })
    }
}

impl Evaluator for RegexEvaluator {
    fn evaluate(&self, input: EvaluationInput<'_>) -> EvaluationResult {
        if self.pattern.is_match(input.body) {
            EvaluationResult {
                is_valid: true,
                error: None,
                error_details: None,
            }
        } else {
            EvaluationResult {
                is_valid: false,
                error: Some(EvaluationError::InvalidFormat),
                error_details: Some(format!(
                    "Output did not match pattern '{}': {}",
                    self.expected_description, input.body
                )),
            }
        }
    }
}