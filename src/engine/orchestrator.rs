// src/engine/orchestrator.rs
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    domain::{EngineError, EvaluationError, EvaluationInput, EvaluationResult, Evaluator, LessonUsage,
		LlmProvider, LlmRequest, LlmResponse, MemoryQuery, TargetExecutor,},
	engine::state::ExecutionState,
	memory::SqliteMemoryRepository, 
    executor::{json::{extract_json_object, strip_code_fences},
        payload::apply_ip_encoding},
    events::{ConsoleObserver, Event, SharedObserver},              
};
#[allow(unused_imports)]
use crate::events::EngineObserver;

pub struct EngineConfig {
    pub max_attempts: u32,
    pub task_type: String,
    pub expected_body_key: Option<String>,
}

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
    observer: SharedObserver,
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
            observer: Arc::new(ConsoleObserver),
        }
    }

    pub fn with_observer(mut self, observer: SharedObserver) -> Self {
        self.observer = observer;
        self
    }

    // -------------------------------------------------------------------
    // Helpers — تنها جایی که event ساخته می‌شه
    // -------------------------------------------------------------------

    #[inline]
    fn emit(&self, event: Event) {
        self.observer.on_event(event);
    }

    /// معادل قبلیِ `🔷 [STATE] ...`. الان به‌جای چاپ، event می‌فرسته.
    #[inline]
    fn state(&self, transition: ExecutionState) {
        self.emit(Event::State { transition });
    }

    /// هر خطایی رو با context مشخص emit می‌کنه. جایگزین چاپ‌های تکراری
    /// `println!("❌ ...")` و `log_transition(Failed)`.
    #[inline]
    fn emit_error(&self, context: &'static str, message: impl Into<String>, fatal: bool) {
        self.emit(Event::Error {
            context,
            message: message.into(),
            fatal,
        });
    }

    // -------------------------------------------------------------------
    // Main loop
    // -------------------------------------------------------------------

    pub async fn execute(
        &self,
        system_prompt: &str,
        user_input: &str,
    ) -> Result<(String, LlmResponse), EngineError> {
        // ۱. ایجاد رکورد Execution
        let execution_id = self
            .memory_repo
            .create_execution(&self.config.task_type, user_input)
            .await
            .map_err(|e| {
                self.emit_error(
                    "database",
                    format!("create_execution failed: {}", e),
                    true,
                );
                EngineError::Database(e.to_string())
            })?;

        let mut current_attempt = 1;
        let mut accumulated_lessons: Vec<AppliedLesson> = Vec::new();

        // ۲. بازیابی اولیه‌ی حافظه
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

        self.state(ExecutionState::Preparing);

        loop {
            if current_attempt > self.config.max_attempts {
                self.emit_error(
                    "engine",
                    format!("Exceeded max attempts ({})", self.config.max_attempts),
                    true,
                );
                self.state(ExecutionState::Failed {
                    reason: "Exceeded max attempts".to_string(),
                    attempts_count: self.config.max_attempts,
                });
                return Err(EngineError::MaxAttemptsExceeded {
                    execution_id: execution_id.clone(),
                    attempts: self.config.max_attempts,
                });
            }

            let lesson_ids_used_this_attempt: Vec<String> =
                accumulated_lessons.iter().map(|l| l.id.clone()).collect();

            self.emit(Event::AttemptStarted {
                execution_id: execution_id.clone(),
                attempt: current_attempt,
                max_attempts: self.config.max_attempts,
            });

            self.emit(Event::LessonsInjected {
                count: accumulated_lessons.len(),
                texts: accumulated_lessons.iter().map(|l| l.text.clone()).collect(),
            });

            // ساخت پرامپت
            let dynamic_system_prompt = self.assemble_prompt(system_prompt, &accumulated_lessons);

            let request = LlmRequest {
                system_prompt: dynamic_system_prompt,
                user_input: user_input.to_string(),
                temperature: Some(0.8),
                max_tokens: None,
            };

            // -------------------------------------------------------------------
            // LLM call
            // -------------------------------------------------------------------
            self.state(ExecutionState::Generating);
            let response = match self.provider.generate(&request).await {
                Ok(res) => res,
                Err(err) => {
                    if err.is_retryable() && current_attempt < self.config.max_attempts {
                        self.emit_error(
                            "llm",
                            format!("retryable transport error, will retry: {}", err),
                            false,
                        );
                        current_attempt += 1;
                        continue;
                    }
                    self.emit_error("llm", format!("transport error: {}", err), true);
                    return Err(EngineError::Llm(err));
                }
            };

            self.emit(Event::LlmResponded {
                raw: response.output.clone(),
                latency_ms: response.latency_ms,
            });
            self.state(ExecutionState::Evaluating {
                subject: "model output structure",
                preview: response.output.chars().take(80).collect(),
            });

            // -------------------------------------------------------------------
            // Parse JSON
            // -------------------------------------------------------------------
            let cleaned = extract_json_object(&response.output)
                .unwrap_or_else(|| strip_code_fences(&response.output));

            let payload: serde_json::Value = match serde_json::from_str(&cleaned) {
                Ok(v) => v,
                Err(err) => {
                    self.emit_error(
                        "parse",
                        format!(
                            "JSON parse failed: {}. Extracted (first 200): {}",
                            err,
                            cleaned.chars().take(200).collect::<String>()
                        ),
                        false,
                    );
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

            // -------------------------------------------------------------------
            // Validate body_key
            // -------------------------------------------------------------------
            if let Some(expected_key) = &self.config.expected_body_key {
                if let Some(body) = payload.get("body") {
                    if let Some(obj) = body.as_object() {
                        if !obj.contains_key(expected_key) {
                            let actual_keys: Vec<&String> = obj.keys().collect();
                            self.emit_error(
                                "payload",
                                format!(
                                    "Body missing expected key '{}'. Got keys: {:?}",
                                    expected_key, actual_keys
                                ),
                                false,
                            );
                            let eval_result = EvaluationResult {
                                is_valid: false,
                                error: Some(EvaluationError::Custom("invalid_payload".to_string())),
                                error_details: Some(format!(
                                    "Payload body is missing expected key '{}'. Got keys: {:?}",
                                    expected_key, actual_keys
                                )),
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
                    }
                }
            }

            // -------------------------------------------------------------------
            // IP-encoding transform
            // -------------------------------------------------------------------
            let mut payload = payload;
            let before_str = payload.to_string();
            if let Some(fmt) = apply_ip_encoding(&mut payload) {
                self.emit(Event::PayloadTransformed {
                    before: before_str,
                    after: payload.to_string(),
                    format: format!("{:?}", fmt),
                });
            }

            // -------------------------------------------------------------------
            // Execute
            // -------------------------------------------------------------------
            let payload_str = payload.to_string();
            self.state(ExecutionState::Executing { payload: payload.clone() });
            self.emit(Event::PayloadParsed { payload: payload_str });

            let outcome = match self.executor.execute(&payload).await {
                Ok(o) => o,
                Err(err) => {
                    self.emit_error("execute", format!("executor error: {}", err), false);
                    let eval_result = EvaluationResult {
                        is_valid: false,
                        error: Some(EvaluationError::MissingField),
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

            // -------------------------------------------------------------------
            // Evaluate
            // -------------------------------------------------------------------
            self.state(ExecutionState::Evaluating {
                subject: "real target response",
                preview: outcome.body.chars().take(80).collect(),
            });
            self.emit(Event::TargetResponded {
                status: outcome.status_code,
                body_excerpt: outcome.body.chars().take(300).collect(),
                latency_ms: outcome.latency_ms,
            });

            let eval_result = self.evaluator.evaluate(EvaluationInput {
                body: &outcome.body,
                status_code: outcome.status_code,
                latency_ms: outcome.latency_ms,
            });

            if eval_result.is_valid {
                self.emit(Event::EvaluationPassed);
            } else {
                self.emit(Event::EvaluationFailed {
                    category: eval_result
                        .error
                        .as_ref()
                        .map(|e| e.to_string())
                        .unwrap_or_default(),
                    detail: eval_result.error_details.clone().unwrap_or_default(),
                });
            }

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

            if eval_result.is_valid {
                    self.state(ExecutionState::Completed {
                    final_response: recorded_response.clone(),
                    attempts_count: current_attempt,
                });
                return Ok((execution_id.clone(), recorded_response));
            }

            current_attempt += 1;
        }
    }

    // -------------------------------------------------------------------
    // record_attempt_and_learn — بدون println!، همه‌ش emit
    // -------------------------------------------------------------------

    async fn record_attempt_and_learn(
        &self,
        execution_id: &str,
        attempt_number: u32,
        request: &LlmRequest,
        response: &LlmResponse,
        eval_result: &EvaluationResult,
        lesson_ids_used: &[String],
        accumulated_lessons: &mut Vec<AppliedLesson>,
    ) -> Result<(), EngineError> {
        let attempt_id = self
            .memory_repo
            .record_attempt(execution_id, attempt_number, request, response, eval_result)
            .await
            .map_err(|e| {
                self.emit_error(
                    "database",
                    format!("record_attempt failed: {}", e),
                    true,
                );
                EngineError::Database(e.to_string())
            })?;

        for lesson_id in lesson_ids_used {
            let usage = LessonUsage {
                id: Uuid::new_v4().to_string(),
                lesson_id: lesson_id.clone(),
                execution_id: execution_id.to_string(),
                attempt_id: attempt_id.clone(),
                resulted_in_success: eval_result.is_valid,
            };
            // شکست در usage نباید کل اجرا رو متوقف کنه — ولی event می‌فرستیم
            if let Err(e) = self.memory_repo.record_lesson_usage(&usage).await {
                self.emit_error(
                    "database",
                    format!("record_lesson_usage failed (non-fatal): {}", e),
                    false,
                );
            }
        }

        if eval_result.is_valid {
            return Ok(());
        }

        if let Some(ref err_cat) = eval_result.error {
            self.state(ExecutionState::Reflecting {
                response: response.clone(),
                eval_result: eval_result.clone(),
             });
            let err_details = eval_result
                .error_details
                .as_deref()
                .unwrap_or("No details provided");

            let lesson_text = self.build_lesson_text(err_cat, err_details);

            match self
                .memory_repo
                .save_lesson(&self.config.task_type, err_cat, &lesson_text)
                .await
            {
                Ok(new_lesson_id) => {
                    self.emit(Event::LessonSaved {
                        id: new_lesson_id.clone(),
                        text: lesson_text.clone(),
                    });
                    accumulated_lessons.push(AppliedLesson {
                        id: new_lesson_id,
                        text: lesson_text,
                    });
                }
                Err(e) => {
                    self.emit_error(
                        "database",
                        format!("save_lesson failed (non-fatal): {}", e),
                        false,
                    );
                }
            }
        }

        Ok(())
    }

    // -------------------------------------------------------------------
    // بقیه — بدون تغییر
    // -------------------------------------------------------------------

    fn build_lesson_text(&self, err_cat: &EvaluationError, err_details: &str) -> String {
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
        if err_details.contains("Egress blocked") {
            if self.config.task_type == "ssrf_ip_encoding_bypass" {
                return "AVOID ERROR: Your encoded IP was WRONG — the request went to a different address. \
                        You must ACTUALLY COMPUTE the value, not guess. Do this step by step:\n\
                        1. Split the target IP into four octets: a.b.c.d\n\
                        2. Compute: a*16777216 + b*65536 + c*256 + d\n\
                        3. Write out each multiplication explicitly before adding.\n\
                        4. ALWAYS keep the scheme (http://), the :PORT, and the /path.\n\
                        Do NOT guess a round number. Do the arithmetic."
                    .to_string();
            } else {
                return "AVOID ERROR: You sent the request to a host the lab did not recognize. \
                        You MUST use the EXACT hostname given in the system prompt — do NOT invent \
                        an IP address and do NOT try to encode one. Re-read the prompt and use the \
                        literal host name it specifies (e.g. 'internal-admin:8080')."
                    .to_string();
            }
        }
        if err_details.contains("No connection adapters were found") {
            return "AVOID ERROR: Your URL was missing a valid scheme (http:// or https://). \
                    Always include the full scheme at the start of the URL."
                .to_string();
        }
        if err_details.contains("Name or service not known")
            || err_details.contains("nodename nor servname provided")
        {
            return "AVOID ERROR: The hostname you used could not be resolved. \
                    Use the exact internal service name given in the system prompt, spelled correctly."
                .to_string();
        }
        format!(
            "AVOID ERROR [{}]: Previous attempt was rejected because: '{}'. Try a different payload next time.",
            err_cat, err_details
        )
    }

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