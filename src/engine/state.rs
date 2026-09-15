// src/engine/state.rs
use std::fmt;
use serde_json::Value;
use crate::domain::{EvaluationResult, LlmResponse};

/// وضعیت‌های مختلف چرخه اجرا. این enum صرفاً یک مدل داده نیست؛ orchestrator واقعاً
/// در هر مرحله یک نمونه از این state رو می‌سازه و لاگ می‌کنه (نگاه کن به
/// engine/orchestrator.rs، متد log_transition).
#[derive(Debug, Clone)]
pub enum ExecutionState {
    /// آماده‌سازی پرامپت اولیه و بازیابی حافظه
    Preparing,
    /// ارسال درخواست به LLM
    Generating,
    /// اجرای واقعی payload تولیدشده روی هدف
    Executing { payload: Value },
    /// ارزیابی پاسخ (چه ساختار خروجی مدل، چه پاسخ واقعی هدف)
    Evaluating { response: LlmResponse },
    /// استخراج درس‌آموخته در صورت بروز خطا و آماده‌سازی retry
    Reflecting {
        response: LlmResponse,
        eval_result: EvaluationResult,
    },
    /// پایان موفقیت‌آمیز اجرا
    Completed {
        final_response: LlmResponse,
        attempts_count: u32,
    },
    /// شکست نهایی پس از اتمام حداکثر تلاش‌ها
    Failed {
        reason: String,
        attempts_count: u32,
    },
}

impl fmt::Display for ExecutionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecutionState::Preparing => write!(f, "Preparing (fetching memory, assembling prompt)"),
            ExecutionState::Generating => write!(f, "Generating (calling LLM provider)"),
            ExecutionState::Executing { payload } => write!(f, "Executing payload against target: {}", payload),
            ExecutionState::Evaluating { response } => {
                let preview: String = response.output.chars().take(80).collect();
                write!(f, "Evaluating response: {}", preview)
            }
            ExecutionState::Reflecting { eval_result, .. } => write!(
                f,
                "Reflecting on failure [{}]",
                eval_result
                    .error
                    .as_ref()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            ),
            ExecutionState::Completed { attempts_count, .. } => {
                write!(f, "Completed after {} attempt(s)", attempts_count)
            }
            ExecutionState::Failed { reason, attempts_count } => {
                write!(f, "Failed after {} attempt(s): {}", attempts_count, reason)
            }
        }
    }
}
