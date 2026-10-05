// src/labs/dbox.rs
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::lab::Lab;
use crate::events::{Event, SharedObserver};

pub enum PreflightResult {
    Up,
    Down(String),
}

/// قبل از اجرای هر lab:
/// ۱. اگه auto_manage روشنه و این lab `box` داره: همه‌ی boxهای دیگه‌ای
///    که تو all_labs هستن رو down می‌کنه، بعد dbox خودش رو up.
/// ۲. TCP connect ساده به target.url می‌زنه.
/// ۳. Up یا Down(reason) برمی‌گردونه.
///
/// مختصات کلی:
/// - هیچ resolve مسیری انجام نمی‌شه. `compose_dir` یا مطلقه یا نسبت به CWD.
/// - برنامه هرگز panic نمی‌کنه: هر خطای docker/TCP فقط log می‌شه.
pub async fn preflight(
    lab: &Arc<Lab>,
    all_labs: &[Arc<Lab>],
    auto_manage: bool,
    observer: &SharedObserver,
) -> PreflightResult {
    let spec = lab.spec();
    let my_dbox = spec.dbox.as_ref();
    
    if my_dbox.is_some() && !auto_manage {
        observer.on_event(Event::Error {
            context: "box",
            message: format!(
                "lab '{}' has a box but auto_manage_box=false in aegis.toml — skipping auto-up",
                lab.id()
            ),
            fatal: false,
        });
    }
    println!("[DEBUG] for running docker compose up -d  auto: {}. mybox.is some?:{}", auto_manage, my_dbox.is_some());
        
    // ۱. اگه auto_manage روشنه و این lab dbox داره: بقیه رو down، خودمون رو up
    if auto_manage && my_dbox.is_some() {
        // for debug
        eprintln!("[DEBUG] preflight: auto_manage={}, box={:?}",
            auto_manage,
            my_dbox.map(|b| &b.compose_dir));

        let mc = my_dbox.unwrap();

        // down همه‌ی boxهای دیگه
        for other in all_labs {
            if Arc::ptr_eq(other, lab) {
                continue;
            }
            let os = other.spec();
            if let Some(ob) = os.dbox.as_ref() {
                let dir = expand_tilde(&ob.compose_dir);
                let _ = tokio::process::Command::new("docker")
                    .args(["compose", "down"])
                    .current_dir(&dir)
                    .output()
                    .await;
                observer.on_event(Event::BoxDown {
                    compose_dir: dir.display().to_string(),
                });
            }
        }

        // up خودمون
        let my_dir = expand_tilde(&mc.compose_dir);
        eprintln!("[DEBUG] running docker compose up -d in {:?}", my_dir);
        match tokio::process::Command::new("docker")
            .args(["compose", "up", "-d"])
            .current_dir(&my_dir)
            .output()
            .await
        {
            Ok(o) if o.status.success() => {
                observer.on_event(Event::BoxUp {
                    compose_dir: my_dir.display().to_string(),
                });
            }
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                observer.on_event(Event::Error {
                    context: "box",
                    message: format!("docker compose up failed in {}: {}", my_dir.display(), err),
                    fatal: false,
                });
            }
            Err(e) => {
                observer.on_event(Event::Error {
                    context: "box",
                    message: format!("could not run docker compose: {}", e),
                    fatal: false,
                });
            }
        }

        let wait = mc.wait_secs.max(1);
        tokio::time::sleep(Duration::from_secs(wait)).await;
    }

    // ۲. health check
    let path = my_dbox
        .map(|b| b.health_check_path.as_str())
        .unwrap_or("/");
    health_check(&spec.target.base_url().to_string(), path, 3).await
}

/// TCP connect ساده. کافیه پورت باز باشه — حتی اگه سرور ۴۰۴ بده.
async fn health_check(base_url: &str, path: &str, timeout_secs: u64) -> PreflightResult {
    let full = format!("{}{}", base_url.trim_end_matches('/'), path);
    let Ok(parsed) = url::Url::parse(&full) else {
        return PreflightResult::Down(format!("bad target.url: {}", base_url));
    };
    let host = parsed.host_str().unwrap_or("127.0.0.1").to_string();
    let port = parsed.port_or_known_default().unwrap_or(80);
    let addr = format!("{}:{}", host, port);

    let connect = tokio::net::TcpStream::connect(&addr);
    match tokio::time::timeout(Duration::from_secs(timeout_secs), connect).await {
        Ok(Ok(_)) => PreflightResult::Up,
        Ok(Err(e)) => PreflightResult::Down(format!("tcp {}: {}", addr, e)),
        Err(_) => PreflightResult::Down(format!("tcp {}: timeout", addr)),
    }
}

/// قبل از رفتن به لب بعدی، dbox این lab رو down کن. اگه این lab box
/// نداشت یا auto_manage خاموش بود، هیچ کاری نکن. idempotent.
pub async fn teardown(lab: &Arc<Lab>, auto_manage: bool, observer: &SharedObserver) {
    if !auto_manage {
        return;
    }
    let spec = lab.spec();
    let Some(b) = spec.dbox.as_ref() else {
        return;
    };
    let dir = expand_tilde(&b.compose_dir);
    let _ = tokio::process::Command::new("docker")
        .args(["compose", "down"])
        .current_dir(&dir)
        .output()
        .await;
    observer.on_event(Event::BoxDown {
        compose_dir: dir.display().to_string(),
    });
}

/// `~/foo` و `~` رو با $HOME جایگزین می‌کنه. Rust خودش این کار رو نمی‌کنه.
fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home);
        }
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}