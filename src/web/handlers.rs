use std::path::PathBuf;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json},
};
use serde::Deserialize;
use serde_json::json;

use crate::labs::{lab::Lab, runner};

use super::AppCtx;

pub async fn index() -> Html<&'static str> {
    Html(include_str!("../../static/index.html"))
}

pub async fn list_labs(State(app): State<AppCtx>) -> impl IntoResponse {
    let labs: Vec<_> = app.labs.iter().enumerate().map(|(i, lab)| {
        json!({
            "index": i,
            "id": lab.id(),
            "name": lab.name(),
            "description": lab.description(),
            "url": lab.url(),
            "target": lab.internal_url(),
            "max_attempts": lab.max_attempts(),
            "status": lab.state(),
            "running": lab.is_running(),
        })
    }).collect();
    Json(json!(labs))
}

pub async fn get_lab(
    State(app): State<AppCtx>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    if idx >= app.labs.len() {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response();
    }
    let lab = &app.labs[idx];
    let spec = lab.spec();
    Json(json!({
        "index": idx,
        "id": lab.id(),
        "name": lab.name(),
        "description": lab.description(),
        "url": lab.url(),
        "target": lab.internal_url(),
        "max_attempts": lab.max_attempts(),
        "system_prompt": spec.system_prompt,
        "default_goal": spec.task.default_goal,
        "status": lab.state(),
        "running": lab.is_running(),
    })).into_response()
}

pub async fn get_yaml(
    State(app): State<AppCtx>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    if idx >= app.labs.len() {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let path = app.labs_dir.join(format!("{}.yaml", app.labs[idx].id()));
    match std::fs::read_to_string(&path) {
        Ok(content) => (StatusCode::OK, content).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("read: {}", e)).into_response(),
    }
}

#[derive(Deserialize)]
pub struct SaveYaml {
    pub content: String,
}

/// ذخیره‌ی YAML:
/// ۱. parse کن (رد اگه نامعتبر)
/// ۲. چک کن lab در حال اجرا نباشه + id عوض نشده
/// ۳. روی دیسک بنویس
/// ۴. در حافظه هم اعمال کن — تا تغییرات فوراً اثر کنن، بدون ری‌استارت برنامه
pub async fn save_yaml(
    State(app): State<AppCtx>,
    Path(idx): Path<usize>,
    Json(payload): Json<SaveYaml>,
) -> impl IntoResponse {
    let new_spec: crate::labs::spec::LabSpec = match serde_yaml::from_str(&payload.content) {
        Ok(s) => s,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("invalid YAML: {}", e)).into_response();
        }
    };

    if idx >= app.labs.len() {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let lab = &app.labs[idx];

    if lab.is_running() {
        return (StatusCode::CONFLICT, "cannot edit a running lab").into_response();
    }
    let old_id = lab.id();
    if new_spec.meta.id != old_id {
        return (
            StatusCode::CONFLICT,
            format!(
                "lab id cannot change on save: '{}' → '{}'",
                old_id, new_spec.meta.id
            ),
        )
            .into_response();
    }

    let path = app.labs_dir.join(format!("{}.yaml", old_id));
    if let Err(e) = std::fs::write(&path, &payload.content) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("write: {}", e)).into_response();
    }

    if let Err(e) = lab.replace_spec(new_spec) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
    }

    (StatusCode::OK, "saved").into_response()
}

pub async fn run_lab(
    State(app): State<AppCtx>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    if idx >= app.labs.len() {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }

    let lab = app.labs[idx].clone();

    // قفل global رو همین‌جا، هم‌زمان با پاسخ HTTP، می‌گیریم — نه داخل تسک
    // spawn‌شده — چون باید بلافاصله بدونیم گرفتیمش یا نه تا 200/409 درست
    // برگردونیم.
    if !Lab::try_start(&lab, &app.ctx.run_lock) {
        return (StatusCode::CONFLICT, "another lab is already running").into_response();
    }

    let ctx = app.ctx.clone();
    let default_goal = lab.default_goal();

    tokio::spawn(async move {
        runner::run_locked(&ctx, &lab, Some(default_goal)).await;
    });

    (StatusCode::OK, "started").into_response()
}

pub async fn list_reports(
    State(app): State<AppCtx>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    if idx >= app.labs.len() { return Json(json!([])); }
    let lab_id = app.labs[idx].id();
    let dir = PathBuf::from("reports");
    let mut reports = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with(&format!("{}_", lab_id)) || !name.ends_with(".json") { continue; }
            let mtime = e.metadata().ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64).unwrap_or(0);
            reports.push(json!({
                "file": name,
                "exec_id": name.trim_start_matches(&format!("{}_", lab_id)).trim_end_matches(".json"),
                "mtime": mtime,
            }));
        }
    }
    reports.sort_by(|a, b| b["mtime"].as_u64().cmp(&a["mtime"].as_u64()));
    Json(json!(reports))
}

pub async fn get_report(
    Path(exec_id): Path<String>,
) -> impl IntoResponse {
    let dir = PathBuf::from("reports");
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.contains(&exec_id) && name.ends_with(".json") {
                if let Ok(c) = std::fs::read_to_string(e.path()) {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&c) {
                        return Json(v).into_response();
                    }
                }
            }
        }
    }
    (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response()
}

/// آخرین چند خط لاگ زنده (برای پنل وب). قدیمی‌ترین اول.
pub async fn get_log(State(app): State<AppCtx>) -> impl IntoResponse {
    let s = app.state.lock().unwrap();
    let n = s.log.len();
    let start = n.saturating_sub(200);
    Json(json!(s.log[start..]))
}
