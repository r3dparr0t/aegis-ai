// src/engine/state.rs
use crate::domain::{EvaluationResult, LlmResponse};

/// وضعیت‌های مختلف چرخه اجرا
#[derive(Debug, Clone)]
pub enum ExecutionState {
    /// آماده‌سازی پرامپت اولیه و بازیابی حافظه
    Preparing,
    /// ارسال درخواست به LLM
    Generating,
    /// ارزیابی ساختاری خروجی
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
