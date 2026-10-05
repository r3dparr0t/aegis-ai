// src/web/mod.rs
pub mod state;
pub mod handlers;

use std::{path::PathBuf, sync::{Arc, RwLock}};
use axum::{routing::{get, post}, Router,};

use crate::{
	labs::{Lab, LabContext},
	config::Config
};

#[derive(Clone)]
pub struct AppCtx {
    pub ctx: Arc<LabContext>,
    pub state: state::SharedWebState,
    /// RwLock چون از پنل وب می‌شه lab جدید اضافه کرد (New Lab +).
    pub labs: Arc<RwLock<Vec<Arc<Lab>>>>,
    pub labs_dir: PathBuf,
    pub config: Arc<Config>,
}

pub async fn serve(addr: &str, app: AppCtx) -> Result<(), Box<dyn std::error::Error>> {
    let router = Router::new()
        .route("/", get(handlers::index))
        .route("/api/labs", get(handlers::list_labs))
        .route("/api/lab/:idx/delete", post(handlers::delete_lab))
        .route("/api/lab/new", post(handlers::create_lab))
        .route("/api/lab/:idx", get(handlers::get_lab))
        .route("/api/lab/:idx/yaml", get(handlers::get_yaml).post(handlers::save_yaml))
        .route("/api/lab/:idx/run", post(handlers::run_lab))
        .route("/api/lab/:idx/reports", get(handlers::list_reports))
        .route("/api/report/:exec_id", get(handlers::get_report))
        .route("/api/log", get(handlers::get_log))
        .route("/api/provider", get(handlers::get_provider).post(handlers::set_provider))
        .route("/api/models", get(handlers::list_models))
        .route("/api/run-all", post(handlers::run_all))
        .route("/api/stop", post(handlers::stop_lab))
        .with_state(app);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;
    Ok(())
}
