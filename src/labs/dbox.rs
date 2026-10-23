// src/labs/dbox.rs
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};

use super::lab::Lab;
use crate::events::{Event, SharedObserver};

pub enum PreflightResult {
    AlreadyUp,
    Started,
    Down(String),
}

#[inline]
fn is_cancelled(cancel: &Arc<AtomicBool>) -> bool {
    cancel.load(Ordering::SeqCst)
}

/// ANSI escape codes رو حذف می‌کنه. docker progress bars و رنگ‌ها رو می‌فرستن
/// که تو UI کنسول زشت می‌شن.
fn strip_ansi(s: &str) -> String {
    if !s.contains('\x1b') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut in_esc = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_esc = true;
        } else if in_esc {
            if c.is_ascii_alphabetic() {
                in_esc = false;
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// دستور کوتاه docker که فقط خروجی نهایی‌اش رو لازم داریم (مثل `docker ps`).
#[allow(dead_code)]
async fn run_docker_capture(
    args: &[&str],
    cwd: &std::path::Path,
    timeout_secs: u64,
) -> Result<std::process::Output, String> {
    let mut cmd = tokio::process::Command::new("docker");
    cmd.args(args).current_dir(cwd).kill_on_drop(true);

    let run_fut = cmd.output();
    let timeout_fut = tokio::time::sleep(Duration::from_secs(timeout_secs));
    let label = format!("docker {:?}", args);

    tokio::select! {
        res = run_fut => res.map_err(|e| format!("{} spawn failed: {}", label, e)),
        _ = timeout_fut => Err(format!("{} timed out after {}s", label, timeout_secs)),
    }
}

/// دستور طولانی docker که می‌خوایم خروجی‌اش زنده stream بشه به observer.
/// timeout + cancel-aware. خروجی خط‌به‌خط به‌صورت `BoxOutput` emit میشه.
async fn run_docker_stream(
    args: &[&str],
    cwd: &std::path::Path,
    timeout_secs: u64,
    cancel: Option<&Arc<AtomicBool>>,
    observer: &SharedObserver,
    compose_dir: &str,
    action: &str,
) -> Result<(bool, i32), String> {
    observer.on_event(Event::BoxStarting {
        compose_dir: compose_dir.to_string(),
        action: action.to_string(),
    });

    let mut cmd = tokio::process::Command::new("docker");
    cmd.args(args)
        .current_dir(cwd)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("docker spawn failed: {}", e))?;

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    // reader task برای stdout
    let observer_out = observer.clone();
    let dir_out = compose_dir.to_string();
    let out_task = tokio::spawn(async move {
        if let Some(pipe) = stdout_pipe {
            let mut reader = BufReader::new(pipe).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let clean = strip_ansi(&line);
                if clean.trim().is_empty() { continue; }
                observer_out.on_event(Event::BoxOutput {
                    compose_dir: dir_out.clone(),
                    stream: "stdout".to_string(),
                    line: clean,
                });
            }
        }
    });

    // reader task برای stderr
    let observer_err = observer.clone();
    let dir_err = compose_dir.to_string();
    let err_task = tokio::spawn(async move {
        if let Some(pipe) = stderr_pipe {
            let mut reader = BufReader::new(pipe).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let clean = strip_ansi(&line);
                if clean.trim().is_empty() { continue; }
                observer_err.on_event(Event::BoxOutput {
                    compose_dir: dir_err.clone(),
                    stream: "stderr".to_string(),
                    line: clean,
                });
            }
        }
    });

    let timeout_fut = tokio::time::sleep(Duration::from_secs(timeout_secs));

    let cancel_fut = async {
        match cancel {
            Some(flag) => loop {
                if flag.load(Ordering::SeqCst) { return; }
                tokio::time::sleep(Duration::from_millis(150)).await;
            },
            None => std::future::pending::<()>().await,
        }
    };

    let result = tokio::select! {
        status = child.wait() => {
            status.map_err(|e| format!("wait failed: {}", e))
        }
        _ = timeout_fut => {
            let _ = child.kill().await;
            Err(format!("timed out after {}s", timeout_secs))
        }
        _ = cancel_fut => {
            let _ = child.kill().await;
            Err("cancelled by user".to_string())
        }
    };

    // صبر تا pipeها بسته بشن (با timeout کوتاه برای امنیت)
    let _ = tokio::time::timeout(Duration::from_secs(3), out_task).await;
    let _ = tokio::time::timeout(Duration::from_secs(3), err_task).await;

    match result {
        Ok(status) => {
            let success = status.success();
            let code = status.code().unwrap_or(-1);
            observer.on_event(Event::BoxFinished {
                compose_dir: compose_dir.to_string(),
                action: action.to_string(),
                success,
            });
            Ok((success, code))
        }
        Err(e) => {
            observer.on_event(Event::BoxFinished {
                compose_dir: compose_dir.to_string(),
                action: action.to_string(),
                success: false,
            });
            Err(e)
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// preflight
// ═══════════════════════════════════════════════════════════════

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

    if !auto_manage {
        return match health_check(&base_url, path, 3).await {
            Ok(()) => PreflightResult::AlreadyUp,
            Err(e) => PreflightResult::Down(e),
        };
    }

    if is_cancelled(cancel) {
        return PreflightResult::Down("cancelled before preflight".into());
    }

    let is_persistent = my_dbox.map(|b| b.teardown == "stop").unwrap_or(false);

    // ۰. persistent + reachable → reuse
    if is_persistent {
        if health_check(&base_url, path, 3).await.is_ok() {
            if let Some(mc) = my_dbox {
                let my_dir = expand_tilde(&mc.compose_dir);
                started_boxes.lock().unwrap().remove(&my_dir);
            }
            return PreflightResult::AlreadyUp;
        }
    }

    // ۱. down loop
    let my_dir_for_skip = my_dbox.map(|mc| expand_tilde(&mc.compose_dir));

    let to_down: Vec<PathBuf> = {
        let mut set = started_boxes.lock().unwrap();
        let all: Vec<PathBuf> = set
            .iter()
            .filter(|p| !(is_persistent && Some(*p) == my_dir_for_skip.as_ref()))
            .cloned()
            .collect();
        for p in &all { set.remove(p); }
        all
    };

    for dir in &to_down {
        if is_cancelled(cancel) {
            return PreflightResult::Down("cancelled before cleanup".into());
        }
        let dir_str = dir.display().to_string();
        let _ = run_docker_stream(
            &["compose", "down"],
            dir,
            60,
            Some(cancel),
            observer,
            &dir_str,
            "down",
        )
        .await;
        observer.on_event(Event::BoxDown { compose_dir: dir_str });
    }

    if !to_down.is_empty() {
        wait_for_port_free(&base_url, 10).await;
    }

    if is_cancelled(cancel) {
        return PreflightResult::Down("cancelled after cleanup".into());
    }

    // ۲. port check
    // اگه سرویس بالاست، باید مطمئن بشیم متعلق به همین lab هست (نه یه lab دیگه
    // که هنوز بسته نشده). مالکیت رو از docker label «compose project working_dir»
    // تشخیص می‌دیم.
    if health_check(&base_url, path, 1).await.is_ok() {
        let port = url::Url::parse(&base_url)
            .ok()
            .and_then(|u| u.port_or_known_default())
            .unwrap_or(80);

        let ps_out = run_docker_capture(
            &[
                "ps",
                "--filter", &format!("publish={}", port),
                "--format", "{{.Names}}\t{{.Label \"com.docker.compose.project.working_dir\"}}",
            ],
            std::path::Path::new("."),
            15,
        )
        .await
        .ok();

        let port_containers: Vec<(String, String)> = ps_out
            .as_ref()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter_map(|l| {
                        let mut parts = l.splitn(2, '\t');
                        Some((
                            parts.next()?.to_string(),
                            parts.next().unwrap_or("").to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();

        // اگه هیچ container داکری روی پورت نیست → احتمالاً user دستی بالا آورده.
        if port_containers.is_empty() {
            return PreflightResult::AlreadyUp;
        }

        if let Some(mc) = my_dbox {
            let my_dir_str = normalize_path(&mc.compose_dir);

            // اگه حداقل یکی از containerهای روی پورت از compose_dir ما هست → reuse
            if port_containers
                .iter()
                .any(|(_, dir)| !dir.is_empty() && normalize_path(dir) == my_dir_str)
            {
                return PreflightResult::AlreadyUp;
            }

            // پورت توسط یه lab دیگه گرفته شده → refuse
            let names: Vec<String> = port_containers
                .iter()
                .map(|(n, d)| {
                    format!(
                        "  - {} (compose_dir: {})",
                        n,
                        if d.is_empty() { "?" } else { d.as_str() }
                    )
                })
                .collect();

            return PreflightResult::Down(format!(
                "port {} is up but NOT owned by this lab's compose_dir ('{}').\n\
                 Container(s) on this port:\n{}\n\
                 This usually means another lab's box is still running on this port.\n\
                 Fix: docker ps --filter publish={} --format '{{{{.Names}}}}' | xargs -r docker stop\n\
                 Or make sure the previous lab's box has `teardown: down` in its spec.",
                port, my_dir_str, names.join("\n"), port
            ));
        }

        // lab بدون dbox → مالکیت قابل چک نیست، فرض می‌کنیم user خودش مدیریت می‌کنه
        return PreflightResult::AlreadyUp;
    }

    // ۳. dbox لازمه
    let Some(mc) = my_dbox else {
        return PreflightResult::Down("target server is not reachable (no box to start)".into());
    };

    // ۴. up
    let my_dir = expand_tilde(&mc.compose_dir);
    let my_dir_str = my_dir.display().to_string();

    let up_timeout = mc.wait_secs.max(30) + 300;
    let up_result = run_docker_stream(
        &["compose", "up", "-d"],
        &my_dir,
        up_timeout,
        Some(cancel),
        observer,
        &my_dir_str,
        "up",
    )
    .await;

    let up_ok = match up_result {
        Ok((success, _code)) if success => {
            observer.on_event(Event::BoxUp { compose_dir: my_dir_str.clone() });
            started_boxes.lock().unwrap().insert(my_dir.clone());
            true
        }
        Ok((_, code)) => {
            observer.on_event(Event::Error {
                context: "box",
                message: format!("docker compose up failed in {} (exit {})", my_dir_str, code),
                fatal: false,
            });
            false
        }
        Err(e) => {
            observer.on_event(Event::Error {
                context: "box",
                message: format!("{}: {}", my_dir_str, e),
                fatal: false,
            });
            if is_cancelled(cancel) {
                return PreflightResult::Down("cancelled during box startup".into());
            }
            false
        }
    };

    if !up_ok {
        return PreflightResult::Down(format!(
            "docker compose up failed in '{}'", my_dir_str
        ));
    }

    // ۵. wait
    let wait = mc.wait_secs.max(1);
    let steps = (wait * 10).max(1);
    for _ in 0..steps {
        if is_cancelled(cancel) {
            return PreflightResult::Down("cancelled while waiting for box".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // ۶. setup_commands
    for cmd in &mc.setup_commands {
        if is_cancelled(cancel) {
            return PreflightResult::Down("cancelled during setup".into());
        }

        observer.on_event(Event::SetupCommandStarted { service: cmd.service.clone() });

        let exec_args: Vec<&str> = vec![
            "compose", "exec", "-T", &cmd.service, "bash", "-c", &cmd.command,
        ];

        let run_result = run_docker_stream(
            &exec_args,
            &my_dir,
            cmd.timeout_secs,
            Some(cancel),
            observer,
            &my_dir_str,
            &format!("exec:{}", cmd.service),
        )
        .await;

        let (ok, excerpt) = match run_result {
            Ok((success, _)) => (success, String::new()), // output already streamed as BoxOutput
            Err(e) => (false, e),
        };

        observer.on_event(Event::SetupCommandFinished {
            service: cmd.service.clone(),
            success: ok,
            excerpt,
        });

        if !ok {
            eprintln!("⚠️  setup_command on '{}' failed", cmd.service);
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
    let Ok(parsed) = url::Url::parse(base_url) else { return true; };
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

pub async fn teardown(
    lab: &Arc<Lab>,
    auto_manage: bool,
    observer: &SharedObserver,
    started_boxes: &Arc<std::sync::Mutex<std::collections::HashSet<PathBuf>>>,
) {
    if !auto_manage { return; }
    let spec = lab.spec();
    let Some(b) = spec.dbox.as_ref() else { return; };
    let dir = expand_tilde(&b.compose_dir);
    let dir_str = dir.display().to_string();
    let compose_cmd = if b.teardown == "down" { "down" } else { "stop" };

    let _ = run_docker_stream(
        &["compose", compose_cmd],
        &dir,
        60,
        None,
        observer,
        &dir_str,
        compose_cmd,
    )
    .await;

    started_boxes.lock().unwrap().remove(&dir);
    observer.on_event(Event::BoxDown { compose_dir: dir_str });
}

fn normalize_path(path: &str) -> String {
    let p = expand_tilde(path);
    std::fs::canonicalize(&p)
        .map(|c| c.display().to_string())
        .unwrap_or_else(|_| p.display().to_string())
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