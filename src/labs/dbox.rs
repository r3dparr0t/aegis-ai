// src/labs/dbox.rs
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use super::lab::Lab;
use crate::events::{Event, SharedObserver};

pub enum PreflightResult {
    /// سرور از قبل بالا بود — Aegis مالکش نیست، نباید teardown بزنه.
    AlreadyUp,
    /// Aegis خودش box رو بالا آورد — پس باید بعداً teardown بزنه.
    Started,
    Down(String),
}

/// قبل از اجرای این lab:
/// ۱. boxهای دیگه‌ای که *خودمون* قبلاً بالا آوردیم رو down می‌کنیم.
/// ۲. اگه سرور بالاست → AlreadyUp (مالک نمی‌شیم).
/// ۳. اگه dbox داره و auto_manage روشنه → فقط dbox *خودش* رو up می‌کنه.
///    ⚠️ اگه `docker compose up` شکست بخوره → `Down` برمی‌گردونیم (نه
///    `Started`)، حتی اگه یه چیزی روی پورت جواب بده. این جلوی fuzz کردن
///    هدف اشتباه (کانتینرِ چپ‌مانده از ران قبلی) رو می‌گیره.
///
/// ⚠️ به هیچ box دیگه‌ای که کاربر دستی بالا آورده دست نمی‌زنه.
/// توی حالت auto، قبل از هر لب مطمئن می‌شیم که هیچ container قدیمی
/// روی پورت هدف نمونده. اگه چیزی موند که ما شروعش نکردیم، fail با پیام واضح.
pub async fn preflight(
    lab: &Arc<Lab>,
    _all_labs: &[Arc<Lab>],
    auto_manage: bool,
    cancel: &Arc<std::sync::atomic::AtomicBool>,
    started_boxes: &Arc<std::sync::Mutex<std::collections::HashSet<PathBuf>>>,
    observer: &SharedObserver,
) -> PreflightResult {
    let spec = lab.spec();
    let my_dbox = spec.dbox.as_ref();
    let base_url = spec.target.base_url().to_string();
    let path = my_dbox.map(|b| b.health_check_path.as_str()).unwrap_or("/");

    // ── Manual mode: کاربر خودش مدیریت می‌کنه. فقط health check. ──
    if !auto_manage {
        return match health_check(&base_url, path, 3).await {
            Ok(()) => PreflightResult::AlreadyUp,
            Err(e) => PreflightResult::Down(e),
        };
    }

    // ══ Auto mode: fresh start، هر بار ══

    if cancel.load(Ordering::SeqCst) {
        return PreflightResult::Down("cancelled before preflight".into());
    }

    // ── ۱. همه‌ی boxهایی که ما شروع کردیم رو down کن (شامل این lab) ──
    // قبلاً my_dir رو filter می‌کردیم، که یعنی اگه همین lab از ران قبلی
    // نیمه‌کاره مونده بود، دست‌نخورده می‌موند. الان همه پاک می‌شن.
    let to_down: Vec<PathBuf> = {
        let mut set = started_boxes.lock().unwrap();
        let all: Vec<PathBuf> = set.iter().cloned().collect();
        set.clear();
        all
    };
    for dir in &to_down {
        let _ = tokio::process::Command::new("docker")
            .args(["compose", "down"])
            .current_dir(dir)
            .output()
            .await;
        observer.on_event(Event::BoxDown {
            compose_dir: dir.display().to_string(),
        });
    }

    // ── ۲. صبر کن پورت واقعاً آزاد بشه (docker down فوری release نمی‌کنه) ──
    if !to_down.is_empty() {
        wait_for_port_free(&base_url, 10).await;
    }

    // ── ۳. اگه هنوز پورت بنده، یه container ناشناس اونجاست ──
    if health_check(&base_url, path, 1).await.is_ok() {
        return PreflightResult::Down(format!(
            "port {} is occupied by a container Aegis did NOT start. \
             Run `docker ps` and `docker kill <id>` manually. \
             This usually means a leftover from a previous manual run.",
            base_url
        ));
    }

    // ── ۴. این lab حتماً dbox داره ──
    let Some(mc) = my_dbox else {
        return PreflightResult::Down("target server is not reachable (no box to start)".into());
    };

    // ── ۵. up کن ──
    let my_dir = expand_tilde(&mc.compose_dir);
    let up_ok = match tokio::process::Command::new("docker")
        .args(["compose", "up", "-d"])
        .current_dir(&my_dir)
        .output()
        .await
    {
        Ok(o) if o.status.success() => {
            observer.on_event(Event::BoxUp {
                compose_dir: my_dir.display().to_string(),
            });
            started_boxes.lock().unwrap().insert(my_dir.clone());
            true
        }
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
            observer.on_event(Event::Error {
                context: "box",
                message: format!("docker compose up failed in {}: {}", my_dir.display(), err),
                fatal: false,
            });
            false
        }
        Err(e) => {
            observer.on_event(Event::Error {
                context: "box",
                message: format!("could not run docker compose: {}", e),
                fatal: false,
            });
            false
        }
    };

    if !up_ok {
        return PreflightResult::Down(format!(
            "docker compose up failed in '{}'",
            my_dir.display()
        ));
    }

    // ── ۶. انتظار برای up شدن سرویس ──
    let wait = mc.wait_secs.max(1);
    let steps = (wait * 10).max(1);
    for _ in 0..steps {
        if cancel.load(Ordering::SeqCst) {
            return PreflightResult::Down("cancelled while waiting for box".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    match health_check(&base_url, path, 3).await {
        Ok(()) => PreflightResult::Started,
        Err(e) => PreflightResult::Down(e),
    }
}

/// منتظر آزاد شدن پورت بمون (تا timeout). true اگه آزاد شد.
async fn wait_for_port_free(base_url: &str, timeout_secs: u64) -> bool {
    let Ok(parsed) = url::Url::parse(base_url) else {
        return true;
    };
    let host = parsed.host_str().unwrap_or("127.0.0.1").to_string();
    let port = parsed.port_or_known_default().unwrap_or(80);
    let addr = format!("{}:{}", host, port);

    let steps = (timeout_secs * 10).max(1);
    for _ in 0..steps {
        match tokio::net::TcpStream::connect(&addr).await {
            Ok(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            Err(_) => return true,
        }
    }
    false
}

/// TCP connect ساده.
async fn health_check(base_url: &str, path: &str, timeout_secs: u64) -> Result<(), String> {
    let full = format!("{}{}", base_url.trim_end_matches('/'), path);
    let Ok(parsed) = url::Url::parse(&full) else {
        return Err(format!("bad target.url: {}", base_url));
    };
    let host = parsed.host_str().unwrap_or("127.0.0.1").to_string();
    let port = parsed.port_or_known_default().unwrap_or(80);
    let addr = format!("{}:{}", host, port);

    let connect = tokio::net::TcpStream::connect(&addr);
    match tokio::time::timeout(Duration::from_secs(timeout_secs), connect).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(format!("tcp {}: {}", addr, e)),
        Err(_) => Err(format!("tcp {}: timeout", addr)),
    }
}

/// بعد از اتمام اجرا، dbox این lab رو down کن. idempotent.
pub async fn teardown(
    lab: &Arc<Lab>,
    auto_manage: bool,
    observer: &SharedObserver,
    started_boxes: &Arc<std::sync::Mutex<std::collections::HashSet<PathBuf>>>,
) {
    if !auto_manage {
        return;
    }
    let spec = lab.spec();
    let Some(b) = spec.dbox.as_ref() else {
        return;
    };
    let dir = expand_tilde(&b.compose_dir);
    let compose_cmd = if b.teardown == "down" { "down" } else { "stop" };
    let _ = tokio::process::Command::new("docker")
        .args(["compose", compose_cmd])
        .current_dir(&dir)
        .output()
        .await;

    // ★ حذف از set تا preflight بعدی دوباره down نزنه
    started_boxes.lock().unwrap().remove(&dir);

    observer.on_event(Event::BoxDown {
        compose_dir: dir.display().to_string(),
    });
}

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
