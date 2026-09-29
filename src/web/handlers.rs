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
    Json(json!({
        "index": idx,
        "id": lab.id(),
        "name": lab.name(),
        "description": lab.description(),
        "url": lab.url(),
        "target": lab.internal_url(),
        "max_attempts": lab.max_attempts(),
        "system_prompt": lab.spec.system_prompt,
        "default_goal": lab.spec.task.default_goal,
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

pub async fn save_yaml(
    State(app): State<AppCtx>,
    Path(idx): Path<usize>,
    Json(payload): Json<SaveYaml>,
) -> impl IntoResponse {
    if let Err(e) = serde_yaml::from_str::<crate::labs::spec::LabSpec>(&payload.content) {
        return (StatusCode::BAD_REQUEST, format!("invalid YAML: {}", e)).into_response();
    }
    if idx >= app.labs.len() {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    // نکته: این فقط فایل YAML رو روی دیسک آپدیت می‌کنه. چون Lab.spec از یه
    // بارگذاری قبلی ساخته شده و immutable ـه، این تغییر بعد از ری‌استارت
    // برنامه اعمال می‌شه، نه فوری روی همین Lab در حافظه.
    let path = app.labs_dir.join(format!("{}.yaml", app.labs[idx].id()));
    match std::fs::write(&path, payload.content) {
        Ok(_) => (StatusCode::OK, "saved").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("write: {}", e)).into_response(),
    }
}

pub async fn run_lab(
    State(app): State<AppCtx>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    if idx >= app.labs.len() {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }

    let lab = app.labs[idx].clone(); // Arc<Lab>

    // قفل global رو همین‌جا، هم‌زمان با پاسخ HTTP، می‌گیریم — نه داخل تسک
    // spawn‌شده — چون باید بلافاصله بدونیم گرفتیمش یا نه تا 200/409 درست
    // برگردونیم. اگه این‌جا فقط `is_running()` رو چک می‌کردیم (بدون گرفتن
    // قفل)، دو تا request هم‌زمان می‌تونستن هر دو از این چک رد بشن.
    if !Lab::try_start(&lab, &app.ctx.run_lock) {
        return (StatusCode::CONFLICT, "another lab is already running").into_response();
    }

    let ctx = app.ctx.clone();
    let default_goal = lab.default_goal();

    tokio::spawn(async move {
        // قفل از قبل گرفته شده، پس از `run_locked` استفاده می‌کنیم نه
        // `runner::run_lab` (که خودش دوباره try_start می‌زنه و چون قفل قبلاً
        // گرفته شده، همیشه false می‌گیره و اجرا رو skip می‌کنه).
        runner::run_locked(&ctx, &lab, Some(default_goal)).await;
    });

    (StatusCode::OK, "started").into_response()
}

pub async fn list_reports(
    State(app): State<AppCtx>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    if idx >= app.labs.len() { return Json(json!([])); }
    let lab_id = app.labs[idx].id().to_string();
    let dir = PathBuf::from("reports");
    let mut reports = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with(&lab_id) || !name.ends_with(".json") { continue; }
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
