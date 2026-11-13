//! Owner routes to preview and export a diagnostic report. No agent tool can
//! request one.

use super::catalog::ApiFault;
use crate::application::ports::catalog::EngineError;
use crate::application::{
    engine::Engine,
    observe::diagnostic_report::{self, DiagnosticReceipt},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    routing::get,
};
use hoista_system::paths::StatePaths;
use std::sync::Arc;

const MAX_BODY_BYTES: usize = 4096;
const COLLECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Clone)]
struct ReportState {
    engine: Arc<Engine>,
    paths: StatePaths,
    slots: Arc<tokio::sync::Semaphore>,
}

/// Tracks collection/writes through requester disconnect and daemon shutdown.
pub struct DiagnosticWork {
    slots: Arc<tokio::sync::Semaphore>,
}

impl DiagnosticWork {
    /// Waits for accepted diagnostic work after the listener has stopped admitting requests.
    pub async fn drain(self) {
        let _permit = self.slots.acquire().await;
        self.slots.close();
    }
}

/// Adds the diagnostic report routes, for the person only, and returns what stops them.
pub fn router(engine: Arc<Engine>, paths: StatePaths) -> (Router, DiagnosticWork) {
    let slots = Arc::new(tokio::sync::Semaphore::new(1));
    let owner = DiagnosticWork {
        slots: slots.clone(),
    };
    let router = Router::new()
        .route("/diagnostics", get(preview).post(save))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(ReportState {
            engine,
            paths,
            slots,
        });
    (router, owner)
}

async fn collect(
    state: &ReportState,
) -> Result<
    (
        diagnostic_report::DiagnosticReport,
        tokio::sync::OwnedSemaphorePermit,
    ),
    ApiFault,
> {
    let permit = state.slots.clone().try_acquire_owned().map_err(|_| {
        EngineError::Conflict("A diagnostic report is already running. Retry when it finishes.")
    })?;
    let report = tokio::time::timeout(
        COLLECT_TIMEOUT,
        diagnostic_report::collect(state.engine.repository.as_ref()),
    )
    .await
    .map_err(|_| {
        EngineError::Conflict("Diagnostic collection timed out. No report was saved; retry when local state is available.")
    })?;
    Ok((report, permit))
}

async fn preview(
    State(state): State<ReportState>,
) -> Result<Json<diagnostic_report::DiagnosticReport>, ApiFault> {
    let (report, _permit) = collect(&state).await?;
    Ok(Json(report))
}

async fn save(
    State(state): State<ReportState>,
    body: axum::body::Bytes,
) -> Result<Json<DiagnosticReceipt>, ApiFault> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Empty {}
    if !body.is_empty() {
        let _: Empty = serde_json::from_slice(&body).map_err(|_| {
            EngineError::Invalid("Diagnostic export accepts only an empty JSON object or no body.")
        })?;
    }
    drop(body);
    let (report, permit) = collect(&state).await?;
    let receipt = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        crate::wiring::save_diagnostic_report(&state.paths, &report)
    })
    .await
    .map_err(|_| EngineError::Store)?
    .map_err(|_| {
        EngineError::Conflict("Could not save the report. Check free disk space and state-directory access, and remove old files from its diagnostics directory if 50 reports are already saved.")
    })?;
    Ok(Json(receipt))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shutdown_waits_for_writer_even_after_its_requester_drops_the_join_handle() {
        let slots = Arc::new(tokio::sync::Semaphore::new(1));
        let owner = DiagnosticWork {
            slots: slots.clone(),
        };
        let observer = slots.clone();
        let permit = slots.acquire_owned().await.expect("writer slot");
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let writer = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let _ = wait.recv();
        });
        drop(writer);
        let drain = owner.drain();
        tokio::pin!(drain);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut drain)
                .await
                .is_err()
        );
        release.send(()).expect("writer still owned");
        tokio::time::timeout(std::time::Duration::from_secs(1), drain)
            .await
            .expect("writer drained");
        assert!(
            observer.is_closed(),
            "Late request bodies must not start a new writer after shutdown"
        );
    }
}
