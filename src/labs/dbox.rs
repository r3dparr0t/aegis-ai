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

    // ══ Auto mode ══

    if cancel.load(Ordering::SeqCst) {
        return PreflightResult::Down("cancelled before preflight".into());
    }

    // ── ۰. اگه این lab ذاتاً persistent ـه (teardown: stop) و پورت جواب
    //      می‌ده، از همون استفاده کن. این قبل از down-loop اجرا می‌شه تا
    //      کانتینری که کاربر دستی (یا از ران قبلی) بالا آورده، کشته نشه.
    //
    //      چرا teardown="stop" سیگنال درستیه؟
    //      stop یعنی «بعد از ران هم کانتینر رو نابود نکن». پس این lab
    //      یه وضعیت داره که باید بمونه (مثلاً Drupal نصب‌شده). اگه پورت
    //      اشغاله، یعنی همون وضعیت موجوده — reuse کن.
    let is_persistent = my_dbox
        .map(|b| b.teardown == "stop")
        .unwrap_or(false);

    if is_persistent {
        if health_check(&base_url, path, 3).await.is_ok() {
            // حتی اگه تو started_boxes بود، پاکش کن — این‌بار مال ما نیست
            // و نمی‌خوایم بعداً teardown روش اجرا بشه.
            if let Some(mc) = my_dbox {
                let my_dir = expand_tilde(&mc.compose_dir);
                started_boxes.lock().unwrap().remove(&my_dir);
            }
            return PreflightResult::AlreadyUp;
        }
        // پورت جواب نمی‌ده → persistent ولی خاموشه → up کن (پایین می‌رسیم)
    }

    // ── ۱. boxهایی که ما شروع کردیم رو down کن ──
    // ⚠️ اگه این lab persistent باشه، دایرکتوری خودش رو از لیست حذف می‌کنیم
    // چون down کردنش یعنی از دست دادن state (نصب Drupal).
    let my_dir_for_skip = my_dbox.map(|mc| expand_tilde(&mc.compose_dir));

    let to_down: Vec<PathBuf> = {
        let mut set = started_boxes.lock().unwrap();
        let all: Vec<PathBuf> = set
            .iter()
            .filter(|p| {
                // اگه persistent و همون dir خودمون → رد کن
                !(is_persistent && Some(*p) == my_dir_for_skip.as_ref())
            })
            .cloned()
            .collect();
        for p in &all {
            set.remove(p);
        }
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

    if !to_down.is_empty() {
        wait_for_port_free(&base_url, 10).await;
    }

    // ── ۲. پورت هنوز بنده؟ ──
    /* health_check(&base_url, path, 1).await.is_ok() {
        return PreflightResult::Down(format!(
            "port {} is occupied by a container Aegis did NOT start. \
             If it's a persistent lab you want to keep, set `teardown: stop` \
             in its box spec. Otherwise run `docker ps` and `docker kill <id>`.",
            base_url
        ));
    }*/
    if health_check(&base_url, path, 1).await.is_ok() {
        // لیست container‌هایی که روی این پورت publish شدن
        let port = url::Url::parse(&base_url)
            .ok()
            .and_then(|u| u.port_or_known_default())
            .unwrap_or(80);

        let ps_out = tokio::process::Command::new("docker")
            .args(["ps", "--filter", &format!("publish={}", port), "--format", "{{.ID}}\t{{.Names}}\t{{.Image}}"])
            .output()
            .await
            .ok();

        let containers = ps_out
            .as_ref()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();

        // اگه همون compose_dir خودمون بود، یعنی persistentـه → reuse
        let is_persistent = my_dbox.map(|b| b.teardown == "stop").unwrap_or(false);
        if is_persistent {
            return PreflightResult::AlreadyUp;
        }

        return PreflightResult::Down(format!(
            "port {} is occupied by container(s) Aegis did NOT start:\n{}\n\
            Fix: docker ps --filter publish={} --format '{{{{.ID}}}}' | xargs -r docker kill\n\
            Or set `teardown: stop` in this lab's box spec if you want to keep it across runs.",
            port, containers, port
        ));
    }

    // ── ۳. این lab باید dbox داشته باشه ──
    let Some(mc) = my_dbox else {
        return PreflightResult::Down("target server is not reachable (no box to start)".into());
    };

    // ── ۴. up ──
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
