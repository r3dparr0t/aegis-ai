// src/main.rs
use std::sync::{Arc, Mutex, RwLock};
use std::path::PathBuf;
use sqlx::sqlite::SqlitePoolOptions;

use aegis_ai::{
    events::{ConsoleObserver, SharedObserver},
    input::prompt,
    labs::{self, runner, Lab, LabContext, LabState},
    memory::SqliteMemoryRepository,
    selection::{SelectedProvider, select_provider},
    web::{
        self,
        state::{CompositeObserver, SharedWebState, WebState, WebStateObserver},
    },
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
    let labs = Arc::new(labs_vec);

    // Web state
    let web_state: SharedWebState = Arc::new(Mutex::new(WebState::new()));

    // قفل global
    let run_lock = Arc::new(Mutex::new(None));

    // Observer ترکیبی
    let composite: SharedObserver = Arc::new(CompositeObserver {
        console: ConsoleObserver,
        web: WebStateObserver {
            state: web_state.clone(),
            run_lock: run_lock.clone(),
        },
    });

    // LabContext
    let ctx = Arc::new(LabContext {
        provider: RwLock::new(provider),
        memory_repo,
        prefix,
        observer: composite,
        run_lock: run_lock.clone(),
    });

    // Web server
    let app_ctx = web::AppCtx {
        ctx: ctx.clone(),
        state: web_state.clone(),
        labs: labs.clone(),
        labs_dir: labs_dir.clone(),
    };
    print_banner(&url);

    let _ = open::that(&url);

    let serve_ctx = app_ctx.clone();
    tokio::spawn(async move {
        if let Err(e) = web::serve(HOST, serve_ctx).await {
            eprintln!("Web server error: {}", e);
        }
    });

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // ─── CLI (اختیاری) ───
    // اگه --cli پاس داده شده باشه، این تابع CLI رو اجرا می‌کنه و برمی‌گرده.
    // وگرنه فوراً برمی‌گرده. تو هر دو حالت، main فقط منتظر Ctrl+C می‌مونه.
    cli_mode(labs.clone(), ctx.clone()).await;

    // منتظر Ctrl+C
    tokio::signal::ctrl_c().await.ok();
    println!("\n  👋 Shutting down...");
    Ok(())
}

/// حالت CLI. اگه `--cli` تو args باشه فعال می‌شه، وگرنه فوراً برمی‌گرده.
/// یه lab_id اختیاری هم قبول می‌کنه: `--cli lab1_fetch`.
async fn cli_mode(labs: Arc<Vec<Arc<Lab>>>, ctx: Arc<LabContext>) {
    let args: Vec<String> = std::env::args().collect();
    let cli_mode = args.iter().any(|a| a == "--cli");
    if !cli_mode {
        return;
    }

    let cli_lab: Option<String> = args
        .iter()
        .skip(1)
        .find(|a| !a.starts_with("--"))
        .cloned();

    match cli_lab {
        Some(lab_id) => {
            if let Some(lab) = labs.iter().find(|l| l.id() == lab_id) {
                println!("\n  ▶  Running lab: {}", lab.name());
                runner::run_lab(&ctx, lab, None).await;
            } else {
                eprintln!("❌ Lab '{}' not found", lab_id);
                eprintln!(
                    "   Available: {}",
                    labs.iter().map(|l| l.id()).collect::<Vec<_>>().join(", ")
                );
            }
        }
        None => {
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
                }
            } else if let Some(lab) = labs.iter().find(|l| l.id() == choice) {
                runner::run_lab(&ctx, lab, None).await;
            }
        }
    }

    println!("\n  ✓ CLI done. Web panel still at {}", format!("http://{}", HOST));
}

fn print_banner(url: &str) {
    println!(
        r#"
    _    _____ ____ ___ ____  
   / \  | ____/ ___|_ _/ ___| 
  / _ \ |  _|| |  _ | |\___ \ 
 / ___ \| |__| |_| || | ___) |
/_/   \_\_____\____|___|____/ 
 SSRF Fuzzing Engine 🔴🟡🟢⚪
═════════════════════════════════════════"#
    );
    println!("  🛡  Web panel live at {}", url);
    println!("  ⌨️  CLI mode: cargo run -- --cli [lab_id]");
    println!("  ✓  Ctrl+C to exit\n");
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
