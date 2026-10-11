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

/// یک درس که در یکی از attemptهای این Execution از حافظه بازیابی و استفاده شده.
#[derive(Debug, Serialize)]
pub struct LessonUsageReport {
    pub lesson_id: String,
    pub attempt_number: i64,
    pub resulted_in_success: bool,
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
    /// true اگه تو هر attemptی از این اجرا، حداقل یه hint دستی (از YAML لب)
    /// به system_prompt تزریق شده باشه. وقتی true ـه، «پاس شدن» رو نباید
    /// به‌عنوان توانایی خالص مدل حساب کرد — با کمک حل شده، نه خودش.
    pub hint_used: bool,
    /// true اگه provider این اجرا حین کار، طبق `fallback_strategy` به یه
    /// مدل دیگه سوییچ کرده باشه (مثلاً مدل کوچیک‌تر → بزرگ‌تر).
    pub fallback_used: bool,
    /// همه‌ی درس‌هایی که در طول این Execution از حافظه بازیابی و به یکی از
    /// attemptها تزریق شده‌اند، به همراه اینکه اون attempt خاص پاس شد یا نه.
    /// برای ردیابی اینکه عوض‌شدن نتیجه‌ی یک باکس بین دو اجرا، ناشی از تغییر
    /// لب بوده یا ناشی از درسی بوده که مدل بین این دو اجرا یاد گرفته.
    pub lessons_used: Vec<LessonUsageReport>,
}
