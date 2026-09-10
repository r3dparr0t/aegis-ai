// src/engine/orchestrator.rs
use std::sync::Arc;
use crate::domain::{
    EvaluationError, EvaluationResult, Evaluator, LlmProvider, LlmRequest, LlmResponse, MemoryQuery,
    TargetExecutor,
};
use crate::memory::SqliteMemoryRepository;
use crate::util::strip_code_fences;

pub struct EngineConfig {
    pub max_attempts: u32,
    pub task_type: String,
}

pub struct ExecutionEngine {
    provider: Arc<dyn LlmProvider>,
    executor: Arc<dyn TargetExecutor>,
    evaluator: Arc<dyn Evaluator>,
    memory_repo: SqliteMemoryRepository,
    config: EngineConfig,
}

impl ExecutionEngine {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        executor: Arc<dyn TargetExecutor>,
        evaluator: Arc<dyn Evaluator>,
        memory_repo: SqliteMemoryRepository,
        config: EngineConfig,
    ) -> Self {
        Self {
            provider,
            executor,
            evaluator,
            memory_repo,
            config,
        }
    }

    /// اجرای اصلی حلقه Self-Correction:
    /// Generate (مدل payload می‌سازد) -> Execute (payload واقعاً به هدف زده می‌شود) -> Evaluate (پاسخ واقعی هدف بررسی می‌شود) -> Reflect
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

            // ارسال درخواست به LLM تا payload حمله را بسازد
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

            // ۳. تلاش برای پارس کردن خروجی مدل به‌عنوان payload معتبر برای Executor
            let cleaned = strip_code_fences(&response.output);
            let payload: serde_json::Value = match serde_json::from_str(&cleaned) {
                Ok(v) => v,
                Err(err) => {
                    let eval_result = EvaluationResult {
                        is_valid: false,
                        error: Some(EvaluationError::InvalidJson),
                        error_details: Some(format!("Model output was not valid JSON: {}", err)),
                    };
                    self.record_attempt_and_learn(
                        &execution_id,
                        current_attempt,
                        &request,
                        &response,
                        &eval_result,
                        &mut accumulated_lessons,
                    )
                    .await?;
                    current_attempt += 1;
                    continue;
                }
            };

            // ۴. اجرای واقعی payload روی هدف (Execute)
            let outcome = match self.executor.execute(&payload).await {
                Ok(o) => o,
                Err(err) => {
                    let eval_result = EvaluationResult {
                        is_valid: false,
                        error: Some(EvaluationError::Custom(err.to_string())),
                        error_details: Some(err.to_string()),
                    };
                    self.record_attempt_and_learn(
                        &execution_id,
                        current_attempt,
                        &request,
                        &response,
                        &eval_result,
                        &mut accumulated_lessons,
                    )
                    .await?;
                    current_attempt += 1;
                    continue;
                }
            };

            // ۵. ارزیابی پاسخ *واقعی هدف* (نه خروجی خام مدل)
            let eval_result = self.evaluator.evaluate(&outcome.body);

            // نسخه‌ای از پاسخ برای ثبت/بازگشت که بدنه‌اش پاسخ واقعی هدف است، نه متن خام مدل
            let recorded_response = LlmResponse {
                output: outcome.body.clone(),
                model: response.model.clone(),
                prompt_tokens: response.prompt_tokens,
                completion_tokens: response.completion_tokens,
                latency_ms: outcome.latency_ms,
            };

            self.record_attempt_and_learn(
                &execution_id,
                current_attempt,
                &request,
                &recorded_response,
                &eval_result,
                &mut accumulated_lessons,
            )
            .await?;

            // اگر پاسخ هدف معیار موفقیت را داشت، پایان موفقیت‌آمیز
            if eval_result.is_valid {
                return Ok(recorded_response);
            }

            current_attempt += 1;
        }
    }

    /// ثبت یک تلاش در دیتابیس و در صورت شکست، استخراج و ذخیره‌ی Lesson جدید (فاز Reflecting)
    async fn record_attempt_and_learn(
        &self,
        execution_id: &str,
        attempt_number: u32,
        request: &LlmRequest,
        response: &LlmResponse,
        eval_result: &EvaluationResult,
        accumulated_lessons: &mut Vec<String>,
    ) -> Result<(), String> {
        let _attempt_id = self
            .memory_repo
            .record_attempt(execution_id, attempt_number, request, response, eval_result)
            .await
            .map_err(|e| format!("DB Error recording attempt: {}", e))?;

        if eval_result.is_valid {
            return Ok(());
        }

        if let Some(ref err_cat) = eval_result.error {
            let err_details = eval_result
                .error_details
                .as_deref()
                .unwrap_or("No details provided");

            let lesson_text = format!(
                "AVOID ERROR [{}]: Previous attempt was rejected because: '{}'. Try a different payload next time.",
                err_cat, err_details
            );

            let _ = self
                .memory_repo
                .save_lesson(&self.config.task_type, err_cat, &lesson_text)
                .await;

            accumulated_lessons.push(lesson_text);
        }

        Ok(())
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
