pub mod state;
pub mod handlers;

use std::path::PathBuf;
use std::sync::Arc;
use axum::{routing::{get, post}, Router};
use crate::labs::LabContext;

#[derive(Clone)]
pub struct AppCtx {
    pub ctx: Arc<LabContext>,
    pub state: state::SharedWebState,
    pub labs: Arc<Vec<crate::labs::spec::LabSpec>>,
    pub labs_dir: PathBuf,
}

pub async fn serve(addr: &str, app: AppCtx) -> Result<(), Box<dyn std::error::Error>> {
    let router = Router::new()
        .route("/", get(handlers::index))
        .route("/api/labs", get(handlers::list_labs))
        .route("/api/lab/:idx", get(handlers::get_lab))
        .route("/api/lab/:idx/yaml", get(handlers::get_yaml).post(handlers::save_yaml))
        .route("/api/lab/:idx/run", post(handlers::run_lab))
        .route("/api/lab/:idx/reports", get(handlers::list_reports))
        .route("/api/report/:exec_id", get(handlers::get_report))
        .with_state(app);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;
    Ok(())
}