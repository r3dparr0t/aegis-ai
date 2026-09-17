// src/domain/report.rs
use serde::Serialize;

/// خلاصه‌ی یک تلاش (Attempt) برای گزارش نهایی
#[derive(Debug, Serialize)]
pub struct AttemptReport {
    pub attempt_number: i64,
    pub output: String,
    pub is_valid: bool,
    pub error_category: Option<String>,
    pub error_details: Option<String>,
    pub latency_ms: i64,
}

/// گزارش کامل یک Execution: هدف، همه‌ی تلاش‌ها، و نتیجه‌ی نهایی.
/// این ساختار مستقیماً از جدول‌های executions/attempts/evaluations در SQLite بازسازی می‌شود.
#[derive(Debug, Serialize)]
pub struct ExecutionReport {
    pub execution_id: String,
    pub task_type: String,
    pub goal: String,
    pub created_at: String,
    pub attempts: Vec<AttemptReport>,
    pub success: bool,
}
