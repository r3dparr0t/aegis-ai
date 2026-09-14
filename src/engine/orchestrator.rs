// src/engine/orchestrator.rs
use std::sync::Arc;
use uuid::Uuid;
use crate::domain::{
    EvaluationError, EvaluationResult, Evaluator, LessonUsage, LlmProvider, LlmRequest, LlmResponse,
    MemoryQuery, TargetExecutor,
};
use crate::memory::SqliteMemoryRepository;
use crate::util::strip_code_fences;

pub struct EngineConfig {
    pub max_attempts: u32,
    pub task_type: String,
}

/// یک Lesson که در پرامپت یک attempt خاص تزریق شده؛ id برای ثبت بعدی در lesson_usage نگه داشته می‌شود
struct AppliedLesson {
    id: String,
    text: String,
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
        let mut accumulated_lessons: Vec<AppliedLesson> = Vec::new();

        // ۲. بازیابی اولیه‌ی حافظه (Lessons) پیش از اولین تلاش
        let query = MemoryQuery {
            task_type: self.config.task_type.clone(),
            error_category: None,
            query_text: user_input.to_string(),
            limit: 3,
        };

        if let Ok(candidates) = self.memory_repo.fetch_lessons(&query).await {
            for cand in candidates {
                accumulated_lessons.push(AppliedLesson {
                    id: cand.lesson.id,
                    text: cand.lesson.lesson_learned,
                });
            }
        }

        loop {
            if current_attempt > self.config.max_attempts {
                return Err(format!(
                    "Execution {} failed: Exceeded max attempts ({})",
                    execution_id, self.config.max_attempts
                ));
            }

            // Lessonهایی که دقیقاً در همین attempt به پرامپت تزریق می‌شوند (برای ثبت بعدی در lesson_usage)
            let lesson_ids_used_this_attempt: Vec<String> =
                accumulated_lessons.iter().map(|l| l.id.clone()).collect();

            println!(
                "\n🔁 [Attempt {}/{}] execution_id={}",
                current_attempt, self.config.max_attempts, execution_id
            );
            if accumulated_lessons.is_empty() {
                println!("🧠 No lessons injected (first attempt or no relevant memory).");
            } else {
                println!("🧠 Injecting {} lesson(s) into prompt:", accumulated_lessons.len());
                for l in &accumulated_lessons {
                    println!("   - [{}] {}", &l.id[..8.min(l.id.len())], l.text);
                }
            }

            // ساخت پرامپت نهایی با اعمال درس‌های قبلی در Context Assembler ساده
            let dynamic_system_prompt = self.assemble_prompt(system_prompt, &accumulated_lessons);

            let request = LlmRequest {
                system_prompt: dynamic_system_prompt,
                user_input: user_input.to_string(),
                temperature: Some(0.8), // مقدار بالاتر تا attemptهای مختلف واقعاً payload متفاوت امتحان کنند
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

            println!("📝 Model raw output:\n{}", response.output);

            // ۳. تلاش برای پارس کردن خروجی مدل به‌عنوان payload معتبر برای Executor
            let cleaned = strip_code_fences(&response.output);
            let payload: serde_json::Value = match serde_json::from_str(&cleaned) {
                Ok(v) => v,
                Err(err) => {
                    println!("❌ Not valid JSON: {}", err);
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
                        &lesson_ids_used_this_attempt,
                        &mut accumulated_lessons,
                    )
                    .await?;
                    current_attempt += 1;
                    continue;
                }
            };

            // ۴. اجرای واقعی payload روی هدف (Execute)
            println!("🎯 Sending payload to target: {}", payload);
            let outcome = match self.executor.execute(&payload).await {
                Ok(o) => o,
                Err(err) => {
                    println!("❌ Executor error: {}", err);
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
                        &lesson_ids_used_this_attempt,
                        &mut accumulated_lessons,
                    )
                    .await?;
                    current_attempt += 1;
                    continue;
                }
            };

            // ۵. ارزیابی پاسخ *واقعی هدف* (نه خروجی خام مدل)
            println!(
                "📡 Target responded [{}]: {}",
                outcome.status_code,
                outcome.body.chars().take(300).collect::<String>()
            );
            let eval_result = self.evaluator.evaluate(&outcome.body);
            if eval_result.is_valid {
                println!("✅ Evaluator: PASSED");
            } else {
                println!(
                    "❌ Evaluator: FAILED [{}] - {}",
                    eval_result.error.as_ref().map(|e| e.to_string()).unwrap_or_default(),
                    eval_result.error_details.as_deref().unwrap_or("")
                );
            }

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
                &lesson_ids_used_this_attempt,
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

    /// ثبت یک تلاش در دیتابیس، ثبت اثر Lessonهای استفاده‌شده در همین تلاش (lesson_usage)،
    /// و در صورت شکست، استخراج و ذخیره‌ی Lesson جدید (فاز Reflecting)
    async fn record_attempt_and_learn(
        &self,
        execution_id: &str,
        attempt_number: u32,
        request: &LlmRequest,
        response: &LlmResponse,
        eval_result: &EvaluationResult,
        lesson_ids_used: &[String],
        accumulated_lessons: &mut Vec<AppliedLesson>,
    ) -> Result<(), String> {
        let attempt_id = self
            .memory_repo
            .record_attempt(execution_id, attempt_number, request, response, eval_result)
            .await
            .map_err(|e| format!("DB Error recording attempt: {}", e))?;

        // برای هر Lesson که در پرامپت همین attempt تزریق شده بود، ثبت می‌کنیم که آیا
        // نتیجه‌ی این تلاش موفقیت‌آمیز بود یا نه (پایه‌ی محاسبه‌ی success_rate در آینده)
        for lesson_id in lesson_ids_used {
            let usage = LessonUsage {
                id: Uuid::new_v4().to_string(),
                lesson_id: lesson_id.clone(),
                execution_id: execution_id.to_string(),
                attempt_id: attempt_id.clone(),
                resulted_in_success: eval_result.is_valid,
            };
            // شکست در ثبت usage نباید کل اجرای engine را متوقف کند
            let _ = self.memory_repo.record_lesson_usage(&usage).await;
        }

        if eval_result.is_valid {
            return Ok(());
        }

        if let Some(ref err_cat) = eval_result.error {
            let err_details = eval_result
                .error_details
                .as_deref()
                .unwrap_or("No details provided");

            let lesson_text = self.build_lesson_text(err_cat, err_details);

            if let Ok(new_lesson_id) = self
                .memory_repo
                .save_lesson(&self.config.task_type, err_cat, &lesson_text)
                .await
            {
                println!("💾 Saved new lesson [{}]: {}", &new_lesson_id[..8.min(new_lesson_id.len())], lesson_text);
                accumulated_lessons.push(AppliedLesson {
                    id: new_lesson_id,
                    text: lesson_text,
                });
            }
        }

        Ok(())
    }

    /// از روی متن خام خطا (که اغلب dump یه exception پایتونیه)، در صورت امکان یک درس
    /// *قابل‌عمل* استخراج می‌کند به‌جای برگردوندن خام همون پیام به مدل.
    fn build_lesson_text(&self, err_cat: &EvaluationError, err_details: &str) -> String {
        // الگوی رایج: اتصال به پورت اشتباه (Connection refused + port=N)
        if err_details.contains("Connection refused") {
            if let Some(port) = Self::extract_port(err_details) {
                return format!(
                    "AVOID ERROR: You connected to port {} and got 'Connection refused'. \
                     That port is wrong for this internal service. Re-read the system prompt \
                     for the exact correct port number and include it explicitly in the URL \
                     as host:PORT (e.g. http://internal-admin:CORRECT_PORT/...).",
                    port
                );
            }
        }

        // الگوی رایج: نبود scheme در URL (requests نمی‌تونه آداپتور پیدا کنه)
        if err_details.contains("No connection adapters were found") {
            return "AVOID ERROR: Your URL was missing a valid scheme (http:// or https://). \
                    Always include the full scheme at the start of the URL."
                .to_string();
        }

        // الگوی رایج: خطای DNS/hostname
        if err_details.contains("Name or service not known")
            || err_details.contains("nodename nor servname provided")
        {
            return "AVOID ERROR: The hostname you used could not be resolved. \
                    Use the exact internal service name given in the system prompt, spelled correctly."
                .to_string();
        }

        // پیش‌فرض: همون رفتار قبلی (dump خام خطا)، برای موارد ناشناخته
        format!(
            "AVOID ERROR [{}]: Previous attempt was rejected because: '{}'. Try a different payload next time.",
            err_cat, err_details
        )
    }

    /// استخراج شماره پورت از رشته‌هایی مثل "port=80)" که در exceptionهای requests/urllib3 پایتون رایجه
    fn extract_port(text: &str) -> Option<&str> {
        let idx = text.find("port=")?;
        let after = &text[idx + "port=".len()..];
        let end = after.find(|c: char| !c.is_ascii_digit()).unwrap_or(after.len());
        if end == 0 {
            None
        } else {
            Some(&after[..end])
        }
    }

    /// ترکیب سیستم پرامپت با حافظه درس‌آموخته‌ها (Context Assembler)
    fn assemble_prompt(&self, base_system_prompt: &str, lessons: &[AppliedLesson]) -> String {
        if lessons.is_empty() {
            return base_system_prompt.to_string();
        }

        let lessons_block = lessons
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{}. {}", i + 1, l.text))
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            "{}\n\n### CRITICAL LESSONS FROM PREVIOUS ATTEMPTS (DO NOT REPEAT THESE ERRORS):\n{}",
            base_system_prompt, lessons_block
        )
    }
}
