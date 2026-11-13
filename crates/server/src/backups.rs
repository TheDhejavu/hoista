//! Owner routes for backups: list, save now, restore into an empty Hoista.

use super::catalog::ApiFault;
use crate::application::{engine::Engine, ports::backups::BackupFile};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use std::sync::Arc;

/// `GET /backups`, `POST /backups`, `POST /backups/{name}/restore`.
pub fn router(engine: Arc<Engine>) -> Router {
    Router::new()
        .route("/backups", get(list).post(save))
        .route("/backups/{name}/restore", post(restore))
        .with_state(engine)
}

async fn list(State(engine): State<Arc<Engine>>) -> Result<Json<Vec<BackupFile>>, ApiFault> {
    Ok(Json(engine.data().backups().await?))
}

async fn save(State(engine): State<Arc<Engine>>) -> Result<Json<BackupFile>, ApiFault> {
    Ok(Json(engine.data().save_backup().await?))
}

async fn restore(
    State(engine): State<Arc<Engine>>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, ApiFault> {
    engine.data().restore_backup(&name).await?;
    Ok(Json(
        serde_json::json!({ "restored": true, "mode": "watching" }),
    ))
}
