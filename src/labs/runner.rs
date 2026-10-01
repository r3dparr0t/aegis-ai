// src/labs/runner.rs
use std::sync::Arc;

use crate::{
    domain::LlmProvider,
    engine::ExecutionEngine,
    provider::OllamaProvider,
    //selection::choose_ollama_model,
};

use super::{lab::Lab, run_task, LabContext};

pub async fn run_lab(ctx: &LabContext, lab: &Arc<Lab>, goal: Option<String>) {
    if !Lab::try_start(lab, &ctx.run_lock) {
        eprintln!(
            "⚠️  Lab '{}' skipped — another lab is already running.",
            lab.id()
        );
        return;
    }
    run_locked(ctx, lab, goal).await;
}

pub async fn run_locked(ctx: &LabContext, lab: &Arc<Lab>, goal: Option<String>) {
    let (success, attempts) = match execute_lab(ctx, lab, goal).await {
        Ok(r) => r,
        Err(msg) => {
            eprintln!("❌ Lab '{}' failed to start: {}", lab.id(), msg);
            (false, 0)
        }
    };

    Lab::finish(lab, &ctx.run_lock, success, attempts);
}

async fn execute_lab(
    ctx: &LabContext,
    lab: &Lab,
    goal: Option<String>,
) -> Result<(bool, u32), String> {
    let provider = pick_provider(ctx, lab).await;
    let executor = lab.build_executor()?;
    let evaluator = lab.build_evaluator();
    let task_type = lab.effective_task_type();
    let config = lab.build_config(task_type);

    let system_prompt = lab.render_system_prompt(&ctx.prefix);

    // goal از پارامتر میاد (وب) یا از default خودِ lab (بدون prompt).
    // هیچ‌وقت از stdin نمی‌خونیم — چون همه‌چیز از وب کنترل می‌شه.
    let user_input = goal.unwrap_or_else(|| lab.default_goal());

    let engine = ExecutionEngine::new(
        provider,
        executor,
        evaluator,
        ctx.memory_repo.clone(),
        config,
    )
    .with_observer(ctx.observer.clone());

    Ok(run_task(
        &engine,
        &ctx.memory_repo,
        &ctx.observer,
        lab,
        &system_prompt,
        &user_input,
    )
    .await)
}

/// انتخاب provider. چون دیگه CLI نداریم، منطق «مدل بزرگ‌تر برای این lab» از
/// خود spec خونده می‌شه — نه از prompt. اگه lab بخواد مدل بزرگ‌تر، از
/// `default_bigger_model` استفاده می‌کنه؛ وگرنه provider پیش‌فرض ctx.
async fn pick_provider(ctx: &LabContext, lab: &Lab) -> Arc<dyn LlmProvider> {
    let opts = lab.options();
    if !opts.allow_bigger_model || opts.default_bigger_model.is_empty() {
        return ctx.provider.read().unwrap().clone();
    }

    // اگه provider فعلی Ollama هست، مدل بزرگ‌تر رو از همون سرور می‌گیریم
    let current = ctx.provider.read().unwrap().info();
    if current.kind != "ollama" {
        return ctx.provider.read().unwrap().clone();
    }

    let model = choose_ollama_model_for_lab(&current.base_url, &opts.default_bigger_model).await;
    Arc::new(OllamaProvider::new(current.base_url, model))
}

/// نسخه‌ی ساده‌شده‌ی choose_ollama_model که از stdin نمی‌خونه — فقط اگه مدل
/// تو لیست Ollama باشه برمی‌گردونه، وگرنه همون default رو استفاده می‌کنه.
async fn choose_ollama_model_for_lab(base_url: &str, preferred: &str) -> String {
    let models = crate::selection::fetch_ollama_models(base_url).await;
    if models.iter().any(|m| m == preferred) {
        preferred.to_string()
    } else if !models.is_empty() {
        // اگه مدل ترجیحی نصب نبود، اولین مدل موجود رو برمی‌گردونیم
        models[0].clone()
    } else {
        preferred.to_string()
    }
}