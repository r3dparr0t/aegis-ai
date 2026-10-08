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
    run_task,
    LabContext,
};

pub async fn run_lab(
    ctx: &LabContext,
    lab: &Arc<Lab>,
    all_labs: &[Arc<Lab>],
    goal: Option<String>,
) {
    // توجه: اینجا عمداً چک نمی‌کنیم که ctx.cancel چی‌ئه. تنها جایی که این
    // flag رو false می‌کنه خط اول run_locked()ه — اگه این‌جا هم روش چک
    // می‌کردیم، بعد از اولین Stop، هیچ‌وقت نوبت به اون reset نمی‌رسید (چون
    // هیچ‌وقت run_locked صدا زده نمی‌شد) و کل برنامه برای همیشه قفل می‌موند.
    // try_start/RunLock به‌تنهایی کافیه که دو اجرا هم‌زمان نشن؛ cancel فقط
    // باید اجرای *در حال انجام* رو متوقف کنه، نه جلوی اجراهای بعدی رو بگیره.
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
    // reset ONCE
    ctx.cancel.store(false, std::sync::atomic::Ordering::SeqCst);
    let lab_id = lab.id();

    // ── ۱. preflight ──
    let preflight = dbox::preflight(
        lab,
        all_labs,
        ctx.auto_manage_box,
        &ctx.cancel,
        &ctx.started_boxes,
        &ctx.observer,
    )
    .await;

    // اگه کاربر وسط preflight Stop زد
    if ctx.cancel.load(std::sync::atomic::Ordering::SeqCst) {
        eprintln!("[STOP] lab {} cancelled during preflight", lab.name());
        if matches!(preflight, PreflightResult::Started) {
            dbox::teardown(lab, ctx.auto_manage_box, &ctx.observer).await;
        }
        Lab::finish_untouched(lab, &ctx.run_lock);
        return;
    }

    let (should_fuzz, owns_box) = match preflight {
        PreflightResult::AlreadyUp => {
            ctx.observer.on_event(Event::ServerUp { lab_id: lab_id.clone() });
            (true, false)   // ← مالک نیست
        }
        PreflightResult::Started => {
            ctx.observer.on_event(Event::ServerUp { lab_id: lab_id.clone() });
            (true, true)    // ← مالک‌ـه
        }
        PreflightResult::Down(reason) => {
            ctx.observer.on_event(Event::ServerDown { lab_id: lab_id.clone(), reason: reason.clone() });
            ctx.observer.on_event(Event::LabSkipped { lab_id: lab_id.clone(), reason });
            (false, false)
        }
    };

    // ── ۲. fuzz (اگه server up بود) ──
    let (success, attempts) = if should_fuzz {
        match execute_lab(ctx, lab, goal).await {
            Ok(r) => r,
            Err(msg) => {
                eprintln!("❌ Lab '{}' failed to start: {}", lab_id, msg);
                (false, 0)
            }
        }
    } else {
        (false, 0)
    };

    // ── ۳. teardown فقط اگه خودمون بالا آورده بودیم ──
    if owns_box {
        dbox::teardown(lab, ctx.auto_manage_box, &ctx.observer).await;
        ctx.observer.on_event(Event::ServerDown {
            lab_id: lab_id.clone(),
            reason: "box was stopped after the run".to_string(),
        });
    }
    
    // ── ۴. state ──
    if should_fuzz {
        Lab::finish(lab, &ctx.run_lock, success, attempts);
    } else {
        Lab::finish_untouched(lab, &ctx.run_lock);
    }
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
    let mut config = lab.build_config(task_type);
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
