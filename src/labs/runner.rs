// src/labs/runner.rs
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    domain::{Evaluator, LlmProvider},
    engine::{EngineConfig, ExecutionEngine},
    evaluator::{FlagEvaluator, TimeDelayEvaluator},
    executor::HttpTargetExecutor,   // ← این‌جا
    input::prompt,
    provider::OllamaProvider,
    selection::choose_ollama_model,
};

use super::{spec::{EvaluatorSpec, LabSpec, InternalTargetSpec}, run_task, LabContext};

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
    
    // ★ executor رو این‌جا بساز، از base_url و endpoint همین Lab
    let (base, path) = match spec.target.split_base_path() {
        Some(bp) => bp,
        None => {
            eprintln!("❌ Invalid target.url in lab '{}': {}", spec.meta.id, spec.target.url);
            return;
        }
    };
    
    let executor = Arc::new(HttpTargetExecutor::new(base, vec![path.as_str()]));
        let evaluator: Arc<dyn Evaluator> = match &spec.evaluator {
            EvaluatorSpec::Flag { marker } => Arc::new(FlagEvaluator::new(marker.clone())),
            EvaluatorSpec::TimeDelay { threshold_ms } => {
                Arc::new(TimeDelayEvaluator::new(*threshold_ms))
            }
        };

    // ★ اینجا internal_target رو تو prompt و goal جایگزین می‌کنیم
    let filled_prompt = fill_target_placeholders(&spec.system_prompt, &spec.internal_target);
    let filled_goal   = fill_target_placeholders(&spec.task.default_goal, &spec.internal_target);

    let system_prompt = format!("{}\n\n{}", ctx.prefix, filled_prompt);
    let user_input = prompt(&format!("Goal for {}", spec.meta.name), &filled_goal);

    let config = EngineConfig {
        max_attempts: spec.max_attempts,   // ← از YAML، نه از ctx
        task_type,
        expected_body_key: if spec.target.body_key.is_empty() {
            None
        } else {
            Some(spec.target.body_key.clone())
        },
    };

    let engine = ExecutionEngine::new(
        provider,
        executor,                    // ← نه ctx.executor
        evaluator,
        ctx.memory_repo.clone(),
        config,
    );

    run_task(&engine, &ctx.memory_repo, &ctx.observer, spec, &system_prompt, &user_input).await;
}

fn fill_target_placeholders(text: &str, target: &InternalTargetSpec) -> String {
    text.replace("{{target_url}}",  &target.url())
        .replace("{{target_host}}", &target.host)
        .replace("{{target_port}}", &target.port.to_string())
        .replace("{{target_path}}", &target.path)
}
