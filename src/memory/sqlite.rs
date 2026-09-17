// src/memory/sqlite.rs
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::domain::{
    AttemptReport, EvaluationError, EvaluationResult, ExecutionReport, Lesson, LessonCandidate,
    LessonUsage, LlmRequest, LlmResponse, MemoryQuery,
};

#[derive(Clone)]
pub struct SqliteMemoryRepository {
    pool: SqlitePool,
}

impl SqliteMemoryRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// ثبت شروع یک Execution جدید
    pub async fn create_execution(&self, task_type: &str, input_prompt: &str) -> Result<String, sqlx::Error> {
        let id = Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO executions (id, task_type, input_prompt) VALUES (?, ?, ?)")
            .bind(&id)
            .bind(task_type)
            .bind(input_prompt)
            .execute(&self.pool)
            .await?;

        Ok(id)
    }

    /// ثبت تلاش (Attempt) به همراه ارزیابی خروجی آن
    pub async fn record_attempt(
        &self,
        execution_id: &str,
        attempt_number: u32,
        request: &LlmRequest,
        response: &LlmResponse,
        eval_result: &EvaluationResult,
    ) -> Result<String, sqlx::Error> {
        let attempt_id = Uuid::new_v4().to_string();
        let prompt_tokens = response.prompt_tokens.map(|v| v as i64);
        let completion_tokens = response.completion_tokens.map(|v| v as i64);
        let latency_ms = response.latency_ms as i64;

        // ۱. ثبت تلاش
        sqlx::query(
            r#"
            INSERT INTO attempts (id, execution_id, attempt_number, system_prompt, output, prompt_tokens, completion_tokens, latency_ms)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&attempt_id)
        .bind(execution_id)
        .bind(attempt_number as i64)
        .bind(&request.system_prompt)
        .bind(&response.output)
        .bind(prompt_tokens)
        .bind(completion_tokens)
        .bind(latency_ms)
        .execute(&self.pool)
        .await?;

        // ۲. ثبت نتیجه ارزیابی
        let eval_id = Uuid::new_v4().to_string();
        let error_cat_str = eval_result.error.as_ref().map(|e| e.to_string());

        sqlx::query(
            r#"
            INSERT INTO evaluations (id, attempt_id, is_valid, error_category, error_details)
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(eval_id)
        .bind(&attempt_id)
        .bind(eval_result.is_valid)
        .bind(error_cat_str)
        .bind(&eval_result.error_details)
        .execute(&self.pool)
        .await?;

        Ok(attempt_id)
    }

    /// ذخیره درس جدید استخراج شده
    pub async fn save_lesson(
        &self,
        task_type: &str,
        error_category: &EvaluationError,
        lesson_learned: &str,
    ) -> Result<String, sqlx::Error> {
        let id = Uuid::new_v4().to_string();
        let err_cat = error_category.to_string();

        sqlx::query(
            r#"
            INSERT INTO lessons (id, task_type, error_category, lesson_learned)
            VALUES (?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(task_type)
        .bind(err_cat)
        .bind(lesson_learned)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    /// بازیابی ترکیبی کاندیداهای درس از دیتابیس: ابتدا با task_type/error_category فیلتر می‌شود،
    /// سپس بر اساس همپوشانی کلیدواژه‌ای با query_text امتیازدهی و مرتب‌سازی می‌شود.
    pub async fn fetch_lessons(&self, query: &MemoryQuery) -> Result<Vec<LessonCandidate>, sqlx::Error> {
        let err_cat_filter = query.error_category.as_ref().map(|e| e.to_string());

        // نکته: قبلاً این شرط با OR نوشته شده بود که باعث می‌شد Lessonهای task_typeهای
        // کاملاً نامرتبط هم برگردند اگر error_category تصادفاً یکسان بود. اینجا همیشه
        // در چارچوب task_type می‌مانیم و error_category فقط در صورت وجود، فیلتر اضافه اعمال می‌کند.
        let rows = sqlx::query(
            r#"
            SELECT id, task_type, error_category, lesson_learned
            FROM lessons
            WHERE task_type = ?
              AND (? IS NULL OR error_category = ?)
            ORDER BY created_at DESC
            "#,
        )
        .bind(&query.task_type)
        .bind(err_cat_filter.clone())
        .bind(err_cat_filter)
        .fetch_all(&self.pool)
        .await?;

        // کلیدواژه‌های ساده از query_text برای امتیازدهی شباهت (Keyword Indexing سبک)
        let query_words: Vec<String> = query
            .query_text
            .to_lowercase()
            .split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
            .filter(|w| w.len() > 2)
            .collect();

        let mut candidates: Vec<LessonCandidate> = rows
            .into_iter()
            .map(|r| {
                let id: String = r.get("id");
                let task_type: String = r.get("task_type");
                let error_category_str: String = r.get("error_category");
                let lesson_learned: String = r.get("lesson_learned");

                let err_cat = match error_category_str.as_str() {
                    "invalid_json" => EvaluationError::InvalidJson,
                    "schema_mismatch" => EvaluationError::SchemaMismatch,
                    "missing_field" => EvaluationError::MissingField,
                    "invalid_format" => EvaluationError::InvalidFormat,
                    other => EvaluationError::Custom(other.to_string()),
                };

                let lesson_lower = lesson_learned.to_lowercase();
                let overlap = query_words
                    .iter()
                    .filter(|w| lesson_lower.contains(w.as_str()))
                    .count();

                // پایه‌ی امتیاز ۱.۰ (تعلق به همان task_type) به‌علاوه‌ی نسبت همپوشانی کلیدواژه‌ای
                let score = if query_words.is_empty() {
                    1.0
                } else {
                    1.0 + (overlap as f32 / query_words.len() as f32)
                };

                LessonCandidate {
                    lesson: Lesson {
                        id,
                        task_type,
                        error_category: err_cat,
                        lesson_learned,
                    },
                    score,
                }
            })
            .collect();

        candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        candidates.truncate(query.limit);

        Ok(candidates)
    }

    /// ثبت سابقه استفاده از درس
    pub async fn record_lesson_usage(&self, usage: &LessonUsage) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO lesson_usage (id, lesson_id, execution_id, attempt_id, resulted_in_success)
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(&usage.id)
        .bind(&usage.lesson_id)
        .bind(&usage.execution_id)
        .bind(&usage.attempt_id)
        .bind(usage.resulted_in_success)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// بازسازی کامل یک گزارش اجرا (Execution Report) از روی جدول‌های executions/attempts/evaluations.
    /// برای گزارش‌دهی نهایی به کاربر (خروجی JSON/Markdown)، نه برای مسیر اصلی self-correction.
    pub async fn get_execution_report(&self, execution_id: &str) -> Result<ExecutionReport, sqlx::Error> {
        let exec_row = sqlx::query("SELECT task_type, input_prompt, created_at FROM executions WHERE id = ?")
            .bind(execution_id)
            .fetch_one(&self.pool)
            .await?;

        let task_type: String = exec_row.get("task_type");
        let goal: String = exec_row.get("input_prompt");
        let created_at: String = exec_row.get("created_at");

        let attempt_rows = sqlx::query(
            r#"
            SELECT a.attempt_number, a.output, a.latency_ms, e.is_valid, e.error_category, e.error_details
            FROM attempts a
            JOIN evaluations e ON e.attempt_id = a.id
            WHERE a.execution_id = ?
            ORDER BY a.attempt_number ASC
            "#,
        )
        .bind(execution_id)
        .fetch_all(&self.pool)
        .await?;

        let mut attempts = Vec::with_capacity(attempt_rows.len());
        let mut success = false;

        for row in attempt_rows {
            let is_valid: bool = row.get("is_valid");
            if is_valid {
                success = true;
            }

            attempts.push(AttemptReport {
                attempt_number: row.get("attempt_number"),
                output: row.get("output"),
                is_valid,
                error_category: row.get("error_category"),
                error_details: row.get("error_details"),
                latency_ms: row.get("latency_ms"),
            });
        }

        Ok(ExecutionReport {
            execution_id: execution_id.to_string(),
            task_type,
            goal,
            created_at,
            attempts,
            success,
        })
    }
}
