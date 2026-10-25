// src/labs/runner.rs
use std::sync::Arc;

use crate::{
    domain::LlmProvider,
    engine::ExecutionEngine,
    events::Event,
    provider::OllamaProvider,
};

use super::{
    dbox::{self, PreflightResult},
    lab::Lab,
    run_task, LabContext, TaskOutcome,
};

pub async fn run_lab(
    ctx: &LabContext,
    lab: &Arc<Lab>,
    all_labs: &[Arc<Lab>],
    goal: Option<String>,
) {
    if !Lab::try_start(lab, &ctx.run_lock) {
        eprintln!(
            "⚠️  Lab '{}' skipped — another lab is already running.",
            lab.id()
        );
        return;
    }
    run_locked(ctx, lab, all_labs, goal).await;
}

pub async fn run_locked(
    ctx: &LabContext,
    lab: &Arc<Lab>,
    all_labs: &[Arc<Lab>],
    goal: Option<String>,
) {
    // reset ONCE per lab — نه per اجرا. این‌طوری اگه کاربر وسط loop
    // Stop بزنه، lab بعدی که شروع بشه cancel رو صفر نمی‌کنه (چون loop
    // قبل از صدا زدن run_locked چک می‌کنه).
    ctx.cancel.store(false, std::sync::atomic::Ordering::SeqCst);


    // ★ مقدار auto_manage_box در ابتدای ران snapshot می‌شه.
    //   اگه کاربر وسط ران توگل کنه، روی همین ران اثری نداره
    //   (تصمیم preflight/teardown قفل می‌مونه).
    let auto_manage = ctx.auto_manage_box.load(std::sync::atomic::Ordering::Relaxed);

    let lab_id = lab.id();

    let preflight = dbox::preflight(
        lab,
        all_labs,
        auto_manage,
        &ctx.cancel,
        &ctx.started_boxes,
        &ctx.observer,
    )
    .await;

    if ctx.cancel.load(std::sync::atomic::Ordering::SeqCst) {
        eprintln!("[STOP] lab {} cancelled during preflight", lab.name());
        if matches!(preflight, PreflightResult::Started) {
            dbox::teardown(lab, auto_manage, &ctx.observer, &ctx.started_boxes).await;
        }
        Lab::finish_cancelled(lab, &ctx.run_lock);
        return;
    }

    if let Some(shell) = &lab.spec().shell {
        if let Err(e) = crate::executor::shell::ShellRuntime::new(
            &shell.script_path,
            shell.timeout_secs,
        ) {
            eprintln!("❌ Lab '{}' has invalid shell config: {}", lab.id(), e);
            ctx.observer.on_event(Event::LabSkipped {
                lab_id: lab.id(),
                reason: format!("invalid shell config: {}", e),
            });
            Lab::finish_untouched(lab, &ctx.run_lock);
            return;
        }
    }
    let (should_fuzz, owns_box) = match preflight {
        PreflightResult::AlreadyUp => {
            ctx.observer.on_event(Event::ServerUp { lab_id: lab_id.clone() });
            (true, false)
        }
        PreflightResult::Started => {
            ctx.observer.on_event(Event::ServerUp { lab_id: lab_id.clone() });
            (true, true)
        }
        PreflightResult::Down(reason) => {
            ctx.observer.on_event(Event::ServerDown { lab_id: lab_id.clone(), reason: reason.clone() });
            ctx.observer.on_event(Event::LabSkipped { lab_id: lab_id.clone(), reason });
            (false, false)
        }
    };

    let outcome = if should_fuzz {
        match execute_lab(ctx, lab, goal).await {
            Ok(o) => o,
            Err(msg) => {
                eprintln!("❌ Lab '{}' failed to start: {}", lab_id, msg);
                TaskOutcome::Errored
            }
        }
    } else {
        TaskOutcome::Errored
    };

    if owns_box {
        dbox::teardown(lab, auto_manage, &ctx.observer, &ctx.started_boxes).await;
        ctx.observer.on_event(Event::ServerDown {
            lab_id: lab_id.clone(),
            reason: "box was stopped after the run".to_string(),
        });
    }

    if !should_fuzz {
        Lab::finish_untouched(lab, &ctx.run_lock);
        return;
    }

    match outcome {
        TaskOutcome::Success { attempts } => Lab::finish(lab, &ctx.run_lock, true, attempts),
        TaskOutcome::Failed  { attempts } => Lab::finish(lab, &ctx.run_lock, false, attempts),
        TaskOutcome::Cancelled            => Lab::finish_cancelled(lab, &ctx.run_lock),
        TaskOutcome::Errored              => Lab::finish_untouched(lab, &ctx.run_lock),
    }
}

async fn execute_lab(
    ctx: &LabContext,
    lab: &Lab,
    goal: Option<String>,
) -> Result<TaskOutcome, String> {
    let provider = pick_provider(ctx, lab).await;
    let executor = lab.build_executor()?;
    let evaluator = lab.build_evaluator();
    let task_type = lab.effective_task_type();
   let mut config = lab.build_config(task_type)?;
    config.cancel = Some(ctx.cancel.clone());
    let system_prompt = lab.render_system_prompt(&ctx.prefix);
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
        &ctx.reports_dir,
        lab,
        &system_prompt,
        &user_input,
    )
    .await)
}

async fn pick_provider(ctx: &LabContext, lab: &Lab) -> Arc<dyn LlmProvider> {
    let opts = lab.options();
    if !opts.allow_bigger_model || opts.default_bigger_model.is_empty() {
        return ctx.provider.read().unwrap().clone();
    }
    let current = ctx.provider.read().unwrap().info();
    if current.kind != "ollama" {
        return ctx.provider.read().unwrap().clone();
    }
    let model = opts.default_bigger_model.clone();
    Arc::new(OllamaProvider::new(current.base_url, model))
}