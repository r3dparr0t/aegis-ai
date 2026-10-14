// src/labs/dbox.rs
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::lab::Lab;
use crate::events::{Event, SharedObserver};

pub enum PreflightResult {
    AlreadyUp,
    Started,
    Down(String),
}

/// docker command با timeout + cancel-aware.
///
/// ⚠️ این تنها راه امن برای صدا زدن docker تو این codebase‌ست. بدون این،
/// یه `docker compose up` هنگ، کل runner رو برای همیشه معلق میکنه — چون
/// run_lock آزاد نمیشه، UI forever "running" میمونه، و Stop بیاثره.
///
/// - `timeout_secs`: سقف زمانی مطلق. طولانیتر از این، ارور برمیگردونه.
/// - `cancel`: اگه Some باشه، بهمحض true شدن flag، پروسه kill میشه.
///   (kill_on_drop(true) مسئول کشتن process وقتی future drop میشه.)
async fn run_docker(
    args: &[&str],
    cwd: &std::path::Path,
    timeout_secs: u64,
    cancel: Option<&Arc<AtomicBool>>,
) -> Result<std::process::Output, String> {
    let mut cmd = tokio::process::Command::new("docker");
    cmd.args(args).current_dir(cwd).kill_on_drop(true);

    let run_fut = cmd.output();
    let timeout_fut = tokio::time::sleep(Duration::from_secs(timeout_secs));

    let label = format!("docker {:?}", args);

    if let Some(flag) = cancel {
        let cancel_fut = async {
            loop {
                if flag.load(Ordering::SeqCst) { return; }
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
        };
        tokio::select! {
            res = run_fut => match res {
                Ok(out) => Ok(out),
                Err(e) => Err(format!("{} spawn failed: {}", label, e)),
            },
            _ = timeout_fut => Err(format!("{} timed out after {}s (killed)", label, timeout_secs)),
            _ = cancel_fut => Err(format!("{} cancelled by user (killed)", label)),
        }
    } else {
        tokio::select! {
            res = run_fut => match res {
                Ok(out) => Ok(out),
                Err(e) => Err(format!("{} spawn failed: {}", label, e)),
            },
            _ = timeout_fut => Err(format!("{} timed out after {}s (killed)", label, timeout_secs)),
        }
    }
}

/// کوتاه‌نویسِ چک cancel.
#[inline]
fn is_cancelled(cancel: &Arc<AtomicBool>) -> bool {
    cancel.load(Ordering::SeqCst)
}

pub async fn preflight(
    lab: &Arc<Lab>,
    _all_labs: &[Arc<Lab>],
    auto_manage: bool,
    cancel: &Arc<AtomicBool>,
    started_boxes: &Arc<std::sync::Mutex<std::collections::HashSet<PathBuf>>>,
    observer: &SharedObserver,
) -> PreflightResult {
    let spec = lab.spec();
    let my_dbox = spec.dbox.as_ref();
    let base_url = spec.target.base_url().to_string();
    let path = my_dbox.map(|b| b.health_check_path.as_str()).unwrap_or("/");

    // ── Manual mode ──
    if !auto_manage {
        return match health_check(&base_url, path, 3).await {
            Ok(()) => PreflightResult::AlreadyUp,
            Err(e) => PreflightResult::Down(e),
        };
    }

    if is_cancelled(cancel) {
        return PreflightResult::Down("cancelled before preflight".into());
    }

    // ── ۰. persistent + reachable → reuse ──
    let is_persistent = my_dbox
        .map(|b| b.teardown == "stop")
        .unwrap_or(false);

    if is_persistent {
        if health_check(&base_url, path, 3).await.is_ok() {
            if let Some(mc) = my_dbox {
                let my_dir = expand_tilde(&mc.compose_dir);
                started_boxes.lock().unwrap().remove(&my_dir);
            }
            return PreflightResult::AlreadyUp;
        }
    }

    // ── ۱. down loop ──
    let my_dir_for_skip = my_dbox.map(|mc| expand_tilde(&mc.compose_dir));

    let to_down: Vec<PathBuf> = {
        let mut set = started_boxes.lock().unwrap();
        let all: Vec<PathBuf> = set
            .iter()
            .filter(|p| {
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
        if is_cancelled(cancel) {
            return PreflightResult::Down("cancelled before cleanup".into());
        }
        // ★ timeout 60s + cancellable
        let _ = run_docker(&["compose", "down"], dir, 60, Some(cancel)).await;
        observer.on_event(Event::BoxDown {
            compose_dir: dir.display().to_string(),
        });
    }

    if !to_down.is_empty() {
        wait_for_port_free(&base_url, 10).await;
    }

    if is_cancelled(cancel) {
        return PreflightResult::Down("cancelled after cleanup".into());
    }

    // ── ۲. port check ──
    if health_check(&base_url, path, 1).await.is_ok() {
        let port = url::Url::parse(&base_url)
            .ok()
            .and_then(|u| u.port_or_known_default())
            .unwrap_or(80);

        let ps_out = run_docker(
            &["ps", "--filter", &format!("publish={}", port), "--format", "{{.ID}}\t{{.Names}}\t{{.Image}}"],
            std::path::Path::new("."),
            15,
            None,
        )
        .await
        .ok();

        let containers = ps_out
            .as_ref()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();

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

    // ── ۳. dbox لازمه ──
    let Some(mc) = my_dbox else {
        return PreflightResult::Down("target server is not reachable (no box to start)".into());
    };

    // ── ۴. up ──
    let my_dir = expand_tilde(&mc.compose_dir);

    // ★ timeout = wait_secs + 300s buffer برای image pull. کاربر همچنان
    //   میتونه وسطش Stop بزنه (چون cancellable).
    let up_timeout = mc.wait_secs.max(30) + 300;
    let up_result = run_docker(&["compose", "up", "-d"], &my_dir, up_timeout, Some(cancel)).await;

    let up_ok = match up_result {
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
                message: e.clone(),
                fatal: false,
            });
            // اگه cancel بود، مستقیم Down با پیام مناسب
            if is_cancelled(cancel) {
                return PreflightResult::Down("cancelled during box startup".into());
            }
            false
        }
    };

    if !up_ok {
        return PreflightResult::Down(format!(
            "docker compose up failed in '{}'",
            my_dir.display()
        ));
    }

    // ── ۵. wait for boot ──
    let wait = mc.wait_secs.max(1);
    let steps = (wait * 10).max(1);
    for _ in 0..steps {
        if is_cancelled(cancel) {
            return PreflightResult::Down("cancelled while waiting for box".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // ── ۶. setup_commands ──
    for cmd in &mc.setup_commands {
        if is_cancelled(cancel) {
            return PreflightResult::Down("cancelled during setup".into());
        }

        observer.on_event(Event::SetupCommandStarted {
            service: cmd.service.clone(),
        });

        // ★ این هم cancellable + timeout
        let exec_args: Vec<&str> = vec![
            "compose", "exec", "-T", &cmd.service, "bash", "-c", &cmd.command,
        ];
        let run_result = run_docker(
            &exec_args,
            &my_dir,
            cmd.timeout_secs,
            Some(cancel),
        )
        .await;

        let (ok, excerpt) = match run_result {
            Ok(o) => {
                let out = String::from_utf8_lossy(&o.stdout);
                let err = String::from_utf8_lossy(&o.stderr);
                let combined = format!("{}{}", out, err);
                (
                    o.status.success(),
                    combined.trim().chars().take(400).collect::<String>(),
                )
            }
            Err(e) => (false, e),
        };

        observer.on_event(Event::SetupCommandFinished {
            service: cmd.service.clone(),
            success: ok,
            excerpt: excerpt.clone(),
        });

        if !ok {
            eprintln!("⚠️  setup_command on '{}' failed: {}", cmd.service, excerpt);
        }
    }

    if is_cancelled(cancel) {
        return PreflightResult::Down("cancelled after setup".into());
    }

    match health_check(&base_url, path, 3).await {
        Ok(()) => PreflightResult::Started,
        Err(e) => PreflightResult::Down(e),
    }
}

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

/// teardown — بعد از اجرا. cancel نمیگیره چون میخوایم حتی اگه کاربر
/// Stop زده باشه، box تمیز بشه. ولی timeout میگیره.
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

    // ★ timeout 60s — اگه docker down هنگ کرد، ما گیر نمیکنیم.
    let _ = run_docker(&["compose", compose_cmd], &dir, 60, None).await;

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