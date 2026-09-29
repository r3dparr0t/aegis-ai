// src/labs/runner.rs
use std::sync::Arc;

use crate::{
    domain::LlmProvider,
    engine::ExecutionEngine,
    input::prompt,
    provider::OllamaProvider,
    selection::choose_ollama_model,
};

use super::{lab::Lab, run_task, LabContext};

/// نسخه‌ی راحت برای CLI: خودش سعی می‌کنه قفل global رو بگیره (`Lab::try_start`)
/// و اگه یه Lab دیگه در حال اجراست، صرف‌نظر می‌کنه.
///
/// از وب استفاده نکن — چون HTTP handler باید بلافاصله (قبل از `tokio::spawn`)
/// بدونه که قفل گرفته شده یا نه تا بتونه ۲۰۰/۴۰۹ درست برگردونه. برای وب از
/// `Lab::try_start` مستقیم در handler + `run_locked` استفاده کن.
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

/// اجرای واقعی، با این فرض که قفل global از قبل گرفته شده (یعنی یه جای دیگه
/// `Lab::try_start` صدا زده و `true` گرفته). این تابع در هر حالتی (موفقیت،
/// شکست، یا خطای ساخت executor) حتماً `Lab::finish` رو صدا می‌زنه — وگرنه
/// قفل global برای همیشه گیر می‌کنه و هیچ Lab دیگه‌ای، نه از CLI و نه از وب،
/// نمی‌تونه اجرا بشه.
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
    let from_cli = goal.is_none();
    let provider = pick_provider(ctx, lab, from_cli).await;
    let executor = lab.build_executor()?;
    let evaluator = lab.build_evaluator();
    let task_type = lab.effective_task_type();
    let config = lab.build_config(task_type);

    let system_prompt = lab.render_system_prompt(&ctx.prefix);
    let user_input = match goal {
        Some(g) => g,
        None => prompt(&format!("Goal for {}", lab.name()), &lab.default_goal()),
    };

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

/// انتخاب provider: اگه lab گزینه‌ی «مدل بزرگ‌تر» رو داشته باشه **و** از CLI
/// اجرا بشه (from_cli == true)، از کاربر بپرس؛ وگرنه provider پیش‌فرض context.
///
/// این شرط باگ قدیمی رو حل می‌کنه: وقتی از وب اجرا می‌شه، هیچ‌وقت نباید
/// prompt روی stdin بزنه چون ترمینالی برای جواب‌دادن وجود نداره.
async fn pick_provider(ctx: &LabContext, lab: &Lab, from_cli: bool) -> Arc<dyn LlmProvider> {
    if !lab.spec.options.allow_bigger_model || !from_cli {
        return ctx.provider.clone();
    }

    let use_bigger = prompt(
        "Use a different (bigger) Ollama model just for this lab? [y/N]",
        "n",
    );
    if !use_bigger.eq_ignore_ascii_case("y") {
        return ctx.provider.clone();
    }

    let url = prompt("Ollama base URL", "http://localhost:11434");
    let default_model = if lab.spec.options.default_bigger_model.is_empty() {
        "qwen2.5:7b"
    } else {
        &lab.spec.options.default_bigger_model
    };
    let model = choose_ollama_model(&url, default_model).await;
    Arc::new(OllamaProvider::new(url, model))
}
