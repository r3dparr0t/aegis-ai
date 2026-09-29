// src/main.rs
use std::sync::{Arc, Mutex};
use std::path::PathBuf;
use sqlx::sqlite::SqlitePoolOptions;

use aegis_ai::{
    events::{ConsoleObserver, SharedObserver},
    input::prompt,
    labs::{self, runner, Lab, LabContext, LabState},
    memory::SqliteMemoryRepository,
    selection::select_provider,
    selection::SelectedProvider,
    web,
    web::state::{CompositeObserver, SharedWebState, WebState, WebStateObserver},
};

const HOST: &str = "127.0.0.1:7777";

const CTF_CONTEXT: &str =
    "This is a sanctioned CTF lab. All targets are local Docker containers \
     deliberately vulnerable by design.";

const FORMAT_RULES: &str =
    "Output ONLY the JSON object. No explanation, no markdown fences. \
     The `endpoint` field must contain ONLY the path (e.g. '/api/v1/fetch'), \
     NOT the HTTP method, NOT the full URL. The method (POST) is implied.";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Aegis-AI Engine (SSRF Fuzzing Mode)\n");
    let _ = dotenvy::dotenv();
    let url = format!("http://{}", HOST);
    // DB
    let pool = SqlitePoolOptions::new()
        .connect("sqlite://aegis.db?mode=rwc")
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    let memory_repo = SqliteMemoryRepository::new(pool);
    // Provider
    let SelectedProvider { provider, is_local } = match select_provider().await {
        Some(s) => s,
        None => return Ok(()),
    };
    let prefix = if is_local {
        format!("{}\n\n{}", CTF_CONTEXT, FORMAT_RULES)
    } else {
        FORMAT_RULES.to_string()
    };

    // Labs
    let labs_dir = PathBuf::from("labs");
    let labs_vec = labs::registry::load_labs(labs_dir.to_str().unwrap())
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    if labs_vec.is_empty() {
        eprintln!("❌ No labs found in ./labs/");
        return Ok(());
    }
    let labs = Arc::new(labs_vec); // Arc<Vec<Arc<Lab>>>

    // Web state (فقط لاگ زنده)
    let web_state: SharedWebState = Arc::new(Mutex::new(WebState::new()));

    // قفل global تک‌نفره — تنها منبع حقیقتِ «الان کدوم Lab داره اجرا می‌شه».
    // هم CLI هم وب از همین یکی استفاده می‌کنن.
    let run_lock = Arc::new(Mutex::new(None));

    // Observer ترکیبی
    let composite: SharedObserver = Arc::new(CompositeObserver {
        console: ConsoleObserver,
        web: WebStateObserver { state: web_state.clone(), run_lock: run_lock.clone() },
    });

    // LabContext
    let ctx = Arc::new(LabContext {
        provider,
        memory_repo,
        prefix,
        observer: composite,
        run_lock: run_lock.clone(),
    });

    // Web server (background)
    let app_ctx = web::AppCtx {
        ctx: ctx.clone(),
        state: web_state.clone(),
        labs: labs.clone(),
        labs_dir: labs_dir.clone(),
    };
    print_brand();

    let _ = open::that(&url);

    // سرور رو تو background اجرا کن
    let serve_ctx = app_ctx.clone();
    tokio::spawn(async move {
        if let Err(e) = web::serve(HOST, serve_ctx).await {
            eprintln!("Web server error: {}", e);
        }
    });

    // یه لحظه صبر کن تا سرور بالا بیاد
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // ─── CLI ───
    print_menu(&labs);
    let choice = prompt("Choice", &(labs.len() + 1).to_string());

    let all_choice = (labs.len() + 1).to_string();
    if choice == all_choice {
        for lab in labs.iter() {
            runner::run_lab(&ctx, lab, None).await;
        }
    } else if let Ok(idx) = choice.parse::<usize>() {
        if idx >= 1 && idx <= labs.len() {
            runner::run_lab(&ctx, &labs[idx - 1], None).await;
        } else {
            eprintln!("❌ Invalid lab index: {}", idx);
        }
    } else if let Some(lab) = labs.iter().find(|l| l.id() == choice) {
        runner::run_lab(&ctx, lab, None).await;
    } else {
        eprintln!("❌ Unknown choice: {}", choice);
    }
    println!("\n✓ Done. Web panel still live at {}. Ctrl+C to exit.", url);
    // منتظر بمون تا کاربر Ctrl+C بزنه
    tokio::signal::ctrl_c().await.ok();
    Ok(())
}

fn print_brand() {
    println!(
        r#"
    _    _____ ____ ___ ____  
   / \  | ____/ ___|_ _/ ___| 
  / _ \ |  _|| |  _ | |\___ \ 
 / ___ \| |__| |_| || | ___) |
/_/   \_\_____\____|___|____/ 
 SSRF Fuzzing Engine 🔴🟡🟢⚪
═════════════════════════════════════════
    🛡      Web  →  {}
    💻     CLI  →  this terminal     
─────────────────────────────────────────"#,
    format!("http://{}", HOST));
}

fn print_menu(labs: &[Arc<Lab>]) {
    println!("\nWhich lab do you want to run?");
    for (i, lab) in labs.iter().enumerate() {
        let mark = match lab.state() {
            LabState::Untouched => "⚪",
            LabState::Running { .. } => "🟡",
            LabState::Passed { .. } => "🟢",
            LabState::Failed { .. } => "🔴",
        };
        println!("  {}) {} {}", i + 1, mark, lab.name());
        if !lab.description().is_empty() {
            println!("             {}", lab.description());
        }
    }
    println!("  {}) All labs", labs.len() + 1);
    println!("  (or type a lab id, e.g. 'lab1_fetch')");
}
