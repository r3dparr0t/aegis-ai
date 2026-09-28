use std::path::PathBuf;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json},
};
use serde::Deserialize;
use serde_json::json;
use crate::labs::runner::run_lab as runner_run_lab;

use super::AppCtx;

pub async fn index() -> Html<&'static str> {
    Html(include_str!("../../static/index.html"))
}

pub async fn list_labs(State(app): State<AppCtx>) -> impl IntoResponse {
    let s = app.state.lock().unwrap();
    let labs: Vec<_> = app.labs.iter().enumerate().map(|(i, lab)| {
        json!({
            "index": i,
            "id": lab.meta.id,
            "name": lab.meta.name,
            "description": lab.meta.description,
            "url": lab.target.url,
            "target": lab.internal_target.url(),
            "max_attempts": lab.max_attempts,
            "status": &s.statuses[i],
        })
    }).collect();
    Json(json!(labs))
}

pub async fn get_lab(
    State(app): State<AppCtx>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    let s = app.state.lock().unwrap();
    if idx >= app.labs.len() {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))).into_response();
    }
    let lab = &app.labs[idx];
    Json(json!({
        "index": idx,
        "id": lab.meta.id,
        "name": lab.meta.name,
        "description": lab.meta.description,
        "url": lab.target.url,
        "target": lab.internal_target.url(),
        "max_attempts": lab.max_attempts,
        "system_prompt": lab.system_prompt,
        "default_goal": lab.task.default_goal,
        "status": &s.statuses[idx],
    })).into_response()
}

pub async fn get_yaml(
    State(app): State<AppCtx>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    if idx >= app.labs.len() {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let path = app.labs_dir.join(format!("{}.yaml", app.labs[idx].meta.id));
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
    let path = app.labs_dir.join(format!("{}.yaml", app.labs[idx].meta.id));
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

    // active_lab رو ست کن
    {
        let mut s = app.state.lock().unwrap();
        s.active_lab = Some(idx);
        s.statuses[idx] = super::state::LabStatus::Running { attempt: 0, max: app.labs[idx].max_attempts };
    }

    let ctx = app.ctx.clone();
    let spec = app.labs[idx].clone();
    let default_goal = spec.task.default_goal.clone();

    tokio::spawn(async move {
        runner_run_lab(&ctx, &spec, Some(default_goal)).await;
    });

    (StatusCode::OK, "started").into_response()
}

pub async fn list_reports(
    State(app): State<AppCtx>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    if idx >= app.labs.len() { return Json(json!([])); }
    let lab_id = &app.labs[idx].meta.id;
    let dir = PathBuf::from("reports");
    let mut reports = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with(lab_id) || !name.ends_with(".json") { continue; }
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