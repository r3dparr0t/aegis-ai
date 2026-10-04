// src/labs/vulhub.rs
use std::sync::Arc;
use std::time::Duration;

use super::lab::Lab;

/// قبل از اجرای یه lab، اگه این lab یه بخش `vulhub` تو YAML داشته باشه و
/// `auto_manage_vulhub` تو aegis.toml true باشه:
///
/// ۱. همه‌ی vulhubهای دیگه (از all_labs) رو `docker compose down` می‌کنه
///    — چون همه‌ی imageهای vulhub پیش‌فرض رو یه پورت (معمولاً 8080) میاد،
///    بدون این کار، فقط یکیشون درست جواب می‌ده.
/// ۲. vulhub خود این lab رو `docker compose up -d` می‌کنه.
/// ۳. `wait_secs` ثانیه صبر می‌کنه تا سرویس آماده بشه.
///
/// اگه docker نبود یا خطا داد، فقط warning می‌ده و lab ادامه پیدا می‌کنه
/// (کاربر ممکنه خودش سرویس رو دستی بالا آورده باشه).
pub async fn ensure_running(
    lab: &Arc<Lab>,               // ← از &Lab به &Arc<Lab>
    all_labs: &[Arc<Lab>],
    enabled: bool,
    vulhub_root: Option<&std::path::Path>,
) {
    if !enabled {
        return;
    }

    let my_spec = lab.spec();
    let Some(my_vulhub) = my_spec.vulhub.as_ref() else {
        return;
    };

    let resolve = |dir: &str| -> std::path::PathBuf {
        let p = std::path::PathBuf::from(dir);
        if p.is_absolute() {
            p
        } else if let Some(root) = vulhub_root {
            root.join(p)
        } else {
            p
        }
    };

    // ۱. down همه‌ی vulhubهای دیگه
    for other in all_labs {
        if Arc::ptr_eq(other, lab) {
            continue;
        }
        let other_spec = other.spec();
        if let Some(ov) = other_spec.vulhub.as_ref() {
            let dir = resolve(&ov.compose_dir);
            let _ = tokio::process::Command::new("docker")
                .args(["compose", "down"])
                .current_dir(&dir)
                .output()
                .await;
        }
    }

    // ۲. up این یکی
    let my_dir = resolve(&my_vulhub.compose_dir);
    let out = tokio::process::Command::new("docker")
        .args(["compose", "up", "-d"])
        .current_dir(&my_dir)
        .output()
        .await;

    match out {
        Ok(o) if o.status.success() => {
            println!("🐳 vulhub up: {}", my_dir.display());
        }
        Ok(o) => {
            eprintln!(
                "⚠️  docker compose up failed in {}: {}",
                my_dir.display(),
                String::from_utf8_lossy(&o.stderr).trim()
            );
        }
        Err(e) => {
            eprintln!("⚠️  could not run docker compose: {}", e);
        }
    }

    // ۳. صبر
    let wait = my_vulhub.wait_secs.max(1);
    tokio::time::sleep(Duration::from_secs(wait)).await;
}