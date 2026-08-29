// src/domain/memory.rs
use crate::domain::execution::EvaluationError;

/// الگوی کوئری از حافظه
#[derive(Debug, Clone)]
pub struct MemoryQuery {
    pub task_type: String,
    pub error_category: Option<EvaluationError>,
    pub query_text: String,
    pub limit: usize,
}

/// ساختار درس‌آموخته
#[derive(Debug, Clone)]
pub struct Lesson {
    pub id: String,
    pub task_type: String,
    pub error_category: EvaluationError,
    pub lesson_learned: String,
}

/// کاندیدای بازیابی‌شده به همراه امتیاز تطابق
#[derive(Debug, Clone)]
pub struct LessonCandidate {
    pub lesson: Lesson,
    pub score: f32,
}

/// ثبت سابقه استفاده از یک درس (برای محاسبه پویای success_rate در آینده)
#[derive(Debug, Clone)]
pub struct LessonUsage {
    pub id: String,
    pub lesson_id: String,
    pub execution_id: String,
    pub attempt_id: String,
    pub resulted_in_success: bool,
}
