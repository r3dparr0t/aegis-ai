// src/engine/orchestrator.rs
use std::sync::Arc;
use crate::domain::{
    EvaluationResult, Evaluator, LessonCandidate, LlmError, LlmProvider, LlmRequest, LlmResponse, MemoryQuery,
};
use crate::memory::SqliteMemoryRepository;

pub struct EngineConfig {
    pub max_attempts: u32,
    pub task_type: String,
}

pub struct ExecutionEngine {
    provider: Arc<dyn LlmProvider>,
    evaluator: Arc<dyn Evaluator>,
    memory_repo: SqliteMemoryRepository,
    config: EngineConfig,
}

impl ExecutionEngine {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        evaluator: Arc<dyn Evaluator>,
        memory_repo: SqliteMemoryRepository,
        config: EngineConfig,
    ) -> Self {
        Self {
            provider,
            evaluator,
            memory_repo,
            config,
        }
    }

    /// اجرای اصلی حلقه Self-Correction
    pub async fn execute(&self, system_prompt: &str, user_input: &str) -> Result<LlmResponse, String> {
        // ۱. ایجاد رکورد Execution جدید در دیتابیس
        let execution_id = self
            .memory_repo
            .create_execution(&self.config.task_type, user_input)
            .await
            .map_err(|e| format!("DB Error creating execution: {}", e))?;

        let mut current_attempt = 1;
        let mut accumulated_lessons: Vec<String> = Vec::new();

        // ۲. بازیابی اولیه‌ی حافظه (Lessons) پیش از اولین تلاش
        let query = MemoryQuery {
            task_type: self.config.task_type.clone(),
            error_category: None,
            query_text: user_input.to_string(),
            limit: 3,
        };

        if let Ok(candidates) = self.memory_repo.fetch_lessons(&query).await {
            for cand in candidates {
                accumulated_lessons.push(cand.lesson.lesson_learned);
            }
        }

        loop {
            if current_attempt > self.config.max_attempts {
                return Err(format!(
                    "Execution {} failed: Exceeded max attempts ({})",
                    execution_id, self.config.max_attempts
                ));
            }

            // ساخت پرامپت نهایی با اعمال درس‌های قبلی در Context Assembler ساده
            let dynamic_system_prompt = self.assemble_prompt(system_prompt, &accumulated_lessons);

            let request = LlmRequest {
                system_prompt: dynamic_system_prompt,
                user_input: user_input.to_string(),
                temperature: Some(0.2), // مقدار کم برای دقت بالادستی
                max_tokens: None,
            };

            // ارسال درخواست به LLM
            let response = match self.provider.generate(&request).await {
                Ok(res) => res,
                Err(err) => {
                    // اگر خطای ارتباطی قابل Retry بود، تلاش مجدد می‌کنیم
                    if err.is_retryable() && current_attempt < self.config.max_attempts {
                        current_attempt += 1;
                        continue;
                    }
                    return Err(format!("LLM Transport Error: {}", err));
                }
            };

            // ارزیابی خروجی توسط Evaluator
            let eval_result = self.evaluator.evaluate(&response.output);

            // ثبت Attempt و نتیجه ارزیابی در دیتابیس
            let _attempt_id = self
                .memory_repo
                .record_attempt(&execution_id, current_attempt, &request, &response, &eval_result)
                .await
                .map_err(|e| format!("DB Error recording attempt: {}", e))?;

            // اگر پاسخ معتبر بود، پایان موفقیت‌آمیز
            if eval_result.is_valid {
                return Ok(response);
            }

            // فاز Reflecting: استخراج خطای معین و ساخت یک Lesson جدید
            if let Some(ref err_cat) = eval_result.error {
                let err_details = eval_result
                    .error_details
                    .as_deref()
                    .unwrap_or("No details provided");

                let lesson_text = format!(
                    "AVOID ERROR [{}]: Previous output was rejected because: '{}'. Ensure proper structure next time.",
                    err_cat, err_details
                );

                // ذخیره درس جدید در حافظه دیتابیس
                let _ = self
                    .memory_repo
                    .save_lesson(&self.config.task_type, err_cat, &lesson_text)
                    .await;

                // تزریق درس جدید به حافظه‌ی اجرای جاری برای تلاش بعدی
                accumulated_lessons.push(lesson_text);
            }

            current_attempt += 1;
        }
    }

    /// ترکیب سیستم پرامپت با حافظه درس‌آموخته‌ها (Context Assembler)
    fn assemble_prompt(&self, base_system_prompt: &str, lessons: &[String]) -> String {
        if lessons.is_empty() {
            return base_system_prompt.to_string();
        }

        let lessons_block = lessons
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{}. {}", i + 1, l))
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            "{}\n\n### CRITICAL LESSONS FROM PREVIOUS ATTEMPTS (DO NOT REPEAT THESE ERRORS):\n{}",
            base_system_prompt, lessons_block
        )
    }
}
