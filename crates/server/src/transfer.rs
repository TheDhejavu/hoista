//! Routes for exporting and inspecting an encrypted transfer on the owner's
//! computer, and for a server receiving one. No agent tool reaches them.

use super::{
    catalog::ApiFault,
    contracts::{
        TransferAbandoned, TransferExportResult, TransferInspection, TransferRecovery,
        TransferRestoreResult,
    },
};
use crate::{
    application::{
        data::transfer::payload::TransferPayload,
        engine::{Actor, Engine},
        ports::catalog::EngineError,
    },
    domain::Secret,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State, rejection::JsonRejection},
    routing::{get, post},
};
use serde::Deserialize;

use std::{path::PathBuf, sync::Arc};

/// Passphrase length in bytes, as `TransferCipher` requires.
const MAX_BODY_BYTES: usize = 8192;
/// A received transfer is base64 (4 bytes for every 3) in a JSON body; this
/// leaves room for the rest of the body.
const RECEIVE_OVERHEAD_BYTES: usize = 64 * 1024;
const PASSPHRASE_BYTES: std::ops::RangeInclusive<usize> = 12..=1024;
#[derive(Clone)]
struct TransferState {
    engine: Arc<Engine>,
    slots: Arc<tokio::sync::Semaphore>,
}

/// Owns accepted export/inspection tasks through requester disconnect and shutdown.
pub struct TransferWork {
    slots: Arc<tokio::sync::Semaphore>,
}

impl TransferWork {
    /// Drains accepted work after transport admission stops, then refuses late requests.
    pub async fn drain(self) {
        let _permit = self.slots.acquire().await;
        self.slots.close();
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    path: PathBuf,
    #[serde(deserialize_with = "crate::application::data::transfer::payload::deserialize_secret")]
    passphrase: Secret,
}

/// Adds owner-only export/inspection routes and their shutdown ownership handle.
pub fn router(engine: Arc<Engine>) -> (Router, TransferWork) {
    let slots = Arc::new(tokio::sync::Semaphore::new(1));
    let owner = TransferWork {
        slots: slots.clone(),
    };
    let router = Router::new()
        .route("/transfer/export", post(export))
        .route("/transfer/inspect", post(inspect))
        .route("/transfer/restore", post(restore))
        .route("/transfer/recovery", get(recovery))
        .route("/transfer/recovery/{id}/abandon", post(abandon))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(TransferState { engine, slots });
    (router, owner)
}

fn input(value: Result<Json<Input>, JsonRejection>) -> Result<Input, ApiFault> {
    let Json(value) = value
        .map_err(|_| EngineError::Invalid("Provide an absolute transfer path and a passphrase."))?;
    if !value.path.is_absolute() || !PASSPHRASE_BYTES.contains(&value.passphrase.expose().len()) {
        return Err(EngineError::Invalid(
            "Use an absolute path and a passphrase of 12–1024 UTF-8 bytes.",
        )
        .into());
    }
    Ok(value)
}

async fn export(
    State(state): State<TransferState>,
    body: Result<Json<Input>, JsonRejection>,
) -> Result<Json<TransferExportResult>, ApiFault> {
    let input = input(body)?;
    let permit = state.slots.clone().try_acquire_owned().map_err(|_| {
        EngineError::Conflict("A transfer operation is running. Wait for it to finish.")
    })?;
    // Once admitted, the transfer runs on its own: a caller that hangs up doesn't stop it.
    let task = tokio::spawn(async move {
        let _permit = permit;
        let payload = state.engine.data().collect_transfer(&Actor::Person).await?;
        let plaintext = payload.encode()?;
        drop(payload);
        let saved = state
            .engine
            .transfer_seal()?
            .seal_to_file(&input.path, input.passphrase, plaintext)
            .await?;
        Ok::<_, EngineError>(TransferExportResult {
            path: saved.path.to_string_lossy().into_owned(),
            bytes: saved.bytes,
            sha256: saved.sha256,
            durable: saved.durable,
            next_step: (!saved.durable)
                .then(|| "The file was published. Inspect this exact file; do not overwrite it or assume export failed.".into()),
        })
    });
    Ok(Json(task.await.map_err(|_| EngineError::Store)??))
}

async fn inspect(
    State(state): State<TransferState>,
    body: Result<Json<Input>, JsonRejection>,
) -> Result<Json<TransferInspection>, ApiFault> {
    let input = input(body)?;
    let permit = state.slots.clone().try_acquire_owned().map_err(|_| {
        EngineError::Conflict("A transfer operation is running. Wait for it to finish.")
    })?;
    let task = tokio::spawn(async move {
        let _permit = permit;
        let plaintext = state
            .engine
            .transfer_seal()?
            .open_file(&input.path, input.passphrase)
            .await?;
        let payload = TransferPayload::decode(&plaintext)?;
        Ok::<_, EngineError>(TransferInspection {
            version: payload.version,
            created_at: payload.created_at,
            has_profile: payload.profile.is_some(),
            accounts: payload.catalog.accounts.len(),
            credentials: payload.credentials.len(),
            projects: payload.catalog.projects.len(),
            resources: payload.catalog.resources.len(),
            ignored_resources: payload.ignored_resources.len(),
            links: payload.catalog.links.len(),
            activity: payload.activity.len(),
            activity_truncated: payload.activity_truncated,
            restored: false,
        })
    });
    Ok(Json(task.await.map_err(|_| EngineError::Store)??))
}

async fn restore(
    State(state): State<TransferState>,
    body: Result<Json<Input>, JsonRejection>,
) -> Result<Json<TransferRestoreResult>, ApiFault> {
    let input = input(body)?;
    let permit = state.slots.clone().try_acquire_owned().map_err(|_| {
        EngineError::Conflict("A transfer operation is running. Wait for it to finish.")
    })?;
    let task = tokio::spawn(async move {
        let _permit = permit;
        let plaintext = state
            .engine
            .transfer_seal()?
            .open_file(&input.path, input.passphrase)
            .await?;
        let payload = TransferPayload::decode(&plaintext)?;
        drop(plaintext);
        let record = state
            .engine
            .data()
            .restore_transfer(&Actor::Person, payload)
            .await?;
        Ok::<_, EngineError>(TransferRestoreResult {
            restored: true,
            mode: crate::domain::model::Mode::Watching,
            restore_id: record.id,
            credentials_staged: record.credential_references.len(),
            provider_access_verified: false,
        })
    });
    Ok(Json(task.await.map_err(|_| EngineError::Store)??))
}

async fn recovery(State(state): State<TransferState>) -> Result<Json<TransferRecovery>, ApiFault> {
    Ok(Json(TransferRecovery {
        attempts: state
            .engine
            .data()
            .transfer_recovery(&Actor::Person)
            .await?,
        limit: 100,
        credential_cleanup:
            "No automatic deletion. Abandoned and interrupted references remain retained.".into(),
    }))
}

async fn abandon(
    State(state): State<TransferState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<TransferAbandoned>, ApiFault> {
    let _permit = state.slots.clone().try_acquire_owned().map_err(|_| {
        EngineError::Conflict(
            "A transfer operation is still running. Wait for it to finish before abandonment.",
        )
    })?;
    state
        .engine
        .data()
        .abandon_transfer(&Actor::Person, &id)
        .await?;
    Ok(Json(TransferAbandoned {
        restore_id: id,
        abandoned: true,
        credentials_deleted: false,
    }))
}

/// A move from the owner's computer: its encrypted export, and the one-time
/// passphrase it was sealed with. Arrives over HTTPS with that computer's key.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Received {
    #[serde(deserialize_with = "crate::application::data::transfer::payload::deserialize_secret")]
    passphrase: Secret,
    /// The export file, base64.
    data: String,
}

/// On a server: `POST /transfer/receive`, restoring a move from the owner's
/// computer into this empty Hoista. Restore refuses a Hoista that already has
/// accounts or projects, so nothing here is ever overwritten.
pub fn receive_router(engine: Arc<Engine>) -> (Router, TransferWork) {
    let max_sealed = engine.transfer_seal().map_or(0, |seal| seal.max_sealed());
    let slots = Arc::new(tokio::sync::Semaphore::new(1));
    let owner = TransferWork {
        slots: slots.clone(),
    };
    let router = Router::new()
        .route("/transfer/receive", post(receive))
        .layer(DefaultBodyLimit::max(
            max_sealed / 3 * 4 + RECEIVE_OVERHEAD_BYTES,
        ))
        .with_state(TransferState { engine, slots });
    (router, owner)
}

async fn receive(
    State(state): State<TransferState>,
    body: Result<Json<Received>, JsonRejection>,
) -> Result<Json<TransferRestoreResult>, ApiFault> {
    use base64::Engine as _;
    let Json(received) =
        body.map_err(|_| EngineError::Invalid("Send the export and its passphrase."))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(received.data.as_bytes())
        .map_err(|_| EngineError::Invalid("The export didn't arrive whole. Try the move again."))?;
    let permit = state.slots.clone().try_acquire_owned().map_err(|_| {
        EngineError::Conflict("A move is already being received. Wait for it to finish.")
    })?;
    let task = tokio::spawn(async move {
        let _permit = permit;
        let plaintext = state
            .engine
            .transfer_seal()?
            .open(bytes, received.passphrase)
            .await?;
        let payload = TransferPayload::decode(&plaintext)?;
        drop(plaintext);
        let record = state
            .engine
            .data()
            .restore_transfer(&Actor::Person, payload)
            .await?;
        Ok::<_, EngineError>(TransferRestoreResult {
            restored: true,
            mode: crate::domain::model::Mode::Watching,
            restore_id: record.id,
            credentials_staged: record.credential_references.len(),
            provider_access_verified: false,
        })
    });
    Ok(Json(task.await.map_err(|_| EngineError::Store)??))
}
