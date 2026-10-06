// src/labs/dbox.rs
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use super::lab::Lab;
use crate::events::{Event, SharedObserver};

pub enum PreflightResult {
    Up,
    Down(String),
}

/// قبل از اجرای این lab:
/// ۱. اگه سرورش بالاست → همون‌جا تموم.
/// ۲. اگه dbox داره و auto_manage روشنه → فقط dbox *خودش* رو up می‌کنه.
/// ۳. صبر، بعد دوباره health check.
///
/// ⚠️ به هیچ box دیگه‌ای دست نمی‌زنه. اگه پورت با lab دیگه‌ای conflict بشه،
/// `docker compose up -d` fail می‌شه و error تو log میاد — کاربر خودش
/// تصمیم می‌گیره که اون یکی رو down کنه یا نه.
pub async fn preflight(
    lab: &Arc<Lab>,
    _all_labs: &[Arc<Lab>],   // ignored — عمداً. هیچ lab دیگه‌ای رو نمی‌بینیم.
    auto_manage: bool,
    cancel: &Arc<std::sync::atomic::AtomicBool>,
    observer: &SharedObserver,
) -> PreflightResult {
    let spec = lab.spec();
    let my_dbox = spec.dbox.as_ref();
    let base_url = spec.target.base_url().to_string();
    let path = my_dbox
        .map(|b| b.health_check_path.as_str())
        .unwrap_or("/");

    // ── ۱. اول چک کن شاید سرور از قبل بالاست ──
    if let PreflightResult::Up = health_check(&base_url, path, 3).await {
        return PreflightResult::Up;
    }

    // ── ۲. اگه box نداریم یا auto_manage خاموشه، فقط Down برگردون ──
    let Some(mc) = my_dbox else {
        return PreflightResult::Down("target server is not reachable (no box to start)".into());
    };
    if !auto_manage {
        return PreflightResult::Down(
            "target server is not reachable (auto_manage_box = false)".into(),
        );
    }

    // ── ۳. cancel check قبل از هر کار سنگین ──
    if cancel.load(Ordering::SeqCst) {
        return PreflightResult::Down("cancelled before box start".into());
    }

    // ── ۴. فقط box خودمون رو up کن ──
    let my_dir = expand_tilde(&mc.compose_dir);
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
                message: format!(
                    "docker compose up failed in {}: {}",
                    my_dir.display(),
                    err
                ),
                fatal: false,
            });
            // همچنان به health check می‌ریم — شاید سرور از قبل نیمه‌بالا بوده
        }
        Err(e) => {
            observer.on_event(Event::Error {
                context: "box",
                message: format!("could not run docker compose: {}", e),
                fatal: false,
            });
        }
    }

    // ── ۵. صبر کن، ولی هر ۱۰۰ms cancel رو چک کن ──
    let wait = mc.wait_secs.max(1);
    let steps = (wait * 10).max(1);
    for _ in 0..steps {
        if cancel.load(Ordering::SeqCst) {
            return PreflightResult::Down("cancelled while waiting for box".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // ── ۶. دوباره چک کن ──
    health_check(&base_url, path, 3).await
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

/// بعد از اتمام اجرا، dbox این lab رو down کن. idempotent.
pub async fn teardown(lab: &Arc<Lab>, auto_manage: bool, observer: &SharedObserver) {
    if !auto_manage {
        return;
    }
    let spec = lab.spec();
    let Some(b) = spec.dbox.as_ref() else {
        return;
    };
    let dir = expand_tilde(&b.compose_dir);
    // teardown از YAML میاد: "stop" (پیش‌فرض) یا "down".
    // هر مقدار دیگه‌ای هم به‌عنوان "stop" رفتار می‌کنه — safe fallback.
    let compose_cmd = if b.teardown == "down" { "down" } else { "stop" };
    let _ = tokio::process::Command::new("docker")
        .args(["compose", compose_cmd])
        .current_dir(&dir)
        .output()
        .await;
    observer.on_event(Event::BoxDown {
        compose_dir: dir.display().to_string(),
    });
}

/// `~/foo` و `~` رو با $HOME جایگزین می‌کنه.
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