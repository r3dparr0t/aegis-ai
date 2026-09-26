// src/labs/runner.rs
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    domain::{Evaluator, LlmProvider},
    engine::{EngineConfig, ExecutionEngine},
    evaluator::{FlagEvaluator, TimeDelayEvaluator},
    input::prompt,
    provider::OllamaProvider,
    selection::choose_ollama_model,
};

use super::{spec::{EvaluatorSpec, LabSpec}, run_task, LabContext};

pub async fn run_lab(ctx: &LabContext, spec: &LabSpec) {
    println!("\n=== 🧪 {} ===", spec.meta.name);
    if !spec.meta.description.is_empty() {
        println!("📖 {}", spec.meta.description);
    }

    let mut task_type = spec.task.task_type.clone();
    if spec.task.fresh_task_type {
        let suffix = &Uuid::new_v4().simple().to_string()[..8];
        task_type = format!("{}_{}", task_type, suffix);
        println!("🆔 Fresh task_type: {}", task_type);
    }

    let provider: Arc<dyn LlmProvider> = if spec.options.allow_bigger_model {
        let use_bigger = prompt(
            "Use a different (bigger) Ollama model just for this lab? [y/N]",
            "n",
        );
        if use_bigger.eq_ignore_ascii_case("y") {
            let url = prompt("Ollama base URL", "http://localhost:11434");
            let default_model = if spec.options.default_bigger_model.is_empty() {
                "qwen2.5:7b"
            } else {
                &spec.options.default_bigger_model
            };
            let model = choose_ollama_model(&url, default_model).await;
            Arc::new(OllamaProvider::new(url, model))
        } else {
            ctx.provider.clone()
        }
    } else {
        ctx.provider.clone()
    };

    let evaluator: Arc<dyn Evaluator> = match &spec.evaluator {
        EvaluatorSpec::Flag { marker } => Arc::new(FlagEvaluator::new(marker.clone())),
        EvaluatorSpec::TimeDelay { threshold_ms } => {
            Arc::new(TimeDelayEvaluator::new(*threshold_ms))
        }
    };

    let system_prompt = format!("{}\n\n{}", ctx.prefix, spec.system_prompt);
    let user_input = prompt(&format!("Goal for {}", spec.meta.name), &spec.task.default_goal);

    let config = EngineConfig {
        max_attempts: ctx.max_attempts,
        task_type,
        expected_body_key: Some(spec.target.body_key.clone()),
    };

    let engine = ExecutionEngine::new(
        provider,
        ctx.executor.clone(),
        evaluator,
        ctx.memory_repo.clone(),
        config,
    );

    run_task(&engine, &ctx.memory_repo, &spec.meta.name, &system_prompt, &user_input).await;
}