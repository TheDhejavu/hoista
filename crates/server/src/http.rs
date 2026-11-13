//! The owner's local HTTP API: size-limited request bodies, an event stream
//! that resumes from saved events, and clean shutdown.

use super::contracts::{ErrorDto, EventDto, ProfileDto};
use crate::{
    application::{StateError, StateStore},
    domain::Profile,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Query, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

/// Event streams open at once.
/// The owner API's requests are small: settings and ids.
const MAX_BODY_BYTES: usize = 4096;
const MAX_STREAMS: usize = 32;
/// How often a stream checks for new activity.
const EVENT_POLL: Duration = Duration::from_millis(250);
const KEEP_ALIVE: Duration = Duration::from_secs(15);
const MAX_FILTER_BYTES: usize = 128;
const MAX_KIND_BYTES: usize = 64;

struct ApiState<S> {
    store: S,
    lifecycle: Option<Arc<super::lifecycle::DaemonLifecycle>>,
    shutdown: CancellationToken,
    streams: Arc<Semaphore>,
    doctor: Arc<Semaphore>,
    engine: Option<Arc<crate::application::engine::Engine>>,
}

impl<S: Clone> Clone for ApiState<S> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            lifecycle: self.lifecycle.clone(),
            shutdown: self.shutdown.clone(),
            streams: self.streams.clone(),
            doctor: self.doctor.clone(),
            engine: self.engine.clone(),
        }
    }
}

/// Builds the local API; bind only to an owner-restricted socket or pipe.
pub fn router<S: StateStore>(store: S, shutdown: CancellationToken) -> Router {
    router_with_engine(store, shutdown, None)
}

/// Builds owner-local diagnostics with optional provider account probes.
pub fn router_with_engine<S: StateStore>(
    store: S,
    shutdown: CancellationToken,
    engine: Option<Arc<crate::application::engine::Engine>>,
) -> Router {
    router_with_lifecycle(store, shutdown, engine, None)
}

/// Builds the daemon owner routes with optional process lifecycle control.
pub fn router_with_lifecycle<S: StateStore>(
    store: S,
    shutdown: CancellationToken,
    engine: Option<Arc<crate::application::engine::Engine>>,
    lifecycle: Option<Arc<super::lifecycle::DaemonLifecycle>>,
) -> Router {
    Router::new()
        .route(
            "/health",
            get(|| async {
                Json(super::contracts::HealthDto {
                    version: crate::VERSION.into(),
                    pid: std::process::id(),
                })
            }),
        )
        .route("/daemon", get(daemon_status::<S>))
        .route("/daemon/restart", post(restart_daemon::<S>))
        .route("/profile", get(profile::<S>).put(update_profile::<S>))
        .route("/events", get(events::<S>))
        .route("/activity", get(activity::<S>))
        .route("/doctor", get(doctor::<S>))
        .fallback(|| async {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "request.not_found",
                "This local API route is not available.",
            )
        })
        .method_not_allowed_fallback(|| async {
            ApiError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "request.method",
                "This method is not supported for this route.",
            )
        })
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(ApiState {
            store,
            lifecycle,
            shutdown,
            engine,
            streams: Arc::new(Semaphore::new(MAX_STREAMS)),
            doctor: Arc::new(Semaphore::new(1)),
        })
}

struct ApiError {
    status: StatusCode,
    body: Box<ErrorDto>,
}

impl ApiError {
    fn new(status: StatusCode, code: &str, message: &str) -> Self {
        Self {
            status,
            body: Box::new(ErrorDto::new(code, message)),
        }
    }
}

impl From<StateError> for ApiError {
    fn from(error: StateError) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "state.unavailable",
            &error.to_string(),
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

async fn daemon_status<S: StateStore>(
    State(state): State<ApiState<S>>,
) -> Result<Json<super::contracts::DaemonStatus>, ApiError> {
    let lifecycle = state.lifecycle.ok_or_else(lifecycle_unavailable)?;
    Ok(Json(lifecycle.status()))
}

fn lifecycle_unavailable() -> ApiError {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "daemon.control_unavailable",
        "This server does not provide daemon lifecycle control.",
    )
}

async fn restart_daemon<S: StateStore>(
    State(state): State<ApiState<S>>,
    input: Result<Json<super::contracts::RestartRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<super::contracts::DaemonStatus>), ApiError> {
    let Json(input) = input.map_err(|_| {
        ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "request.invalid_json",
            "Provide the observed daemon instance ID and optional handoff policy.",
        )
    })?;
    let lifecycle = state.lifecycle.ok_or_else(lifecycle_unavailable)?;
    let status = lifecycle.request_restart(&input).map_err(|message| {
        ApiError::new(StatusCode::CONFLICT, "daemon.instance_changed", message)
    })?;
    Ok((StatusCode::ACCEPTED, Json(status)))
}

async fn profile<S: StateStore>(
    State(state): State<ApiState<S>>,
) -> Result<Json<ProfileDto>, ApiError> {
    state
        .store
        .profile()
        .await?
        .map(|profile| Json(profile.into()))
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "profile.not_found",
                "Set your name and initials to finish local setup.",
            )
        })
}

async fn update_profile<S: StateStore>(
    State(state): State<ApiState<S>>,
    body: Result<Json<ProfileDto>, JsonRejection>,
) -> Result<Json<ProfileDto>, ApiError> {
    let Json(body) = body.map_err(|error| {
        ApiError::new(
            error.status(),
            "request.invalid_json",
            "Send a JSON object with name and initials, no larger than 4096 bytes.",
        )
    })?;
    let profile = Profile::new(body.name, body.initials).map_err(|error| {
        ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "profile.invalid",
            &error.to_string(),
        )
    })?;
    state.store.save_profile(&profile).await?;
    Ok(Json(profile.into()))
}

/// Where a new event stream starts when no `Last-Event-ID` resumes it.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EventsQuery {
    /// `latest` skips committed history and follows only new events.
    after: Option<String>,
}

async fn events<S: StateStore>(
    State(state): State<ApiState<S>>,
    headers: HeaderMap,
    query: Result<Query<EventsQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, ApiError> {
    let invalid_after = || {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "events.invalid_start",
            "Use after=latest, or omit it to replay from the start.",
        )
    };
    let start = match query.map_err(|_| invalid_after())?.0.after.as_deref() {
        None => 0,
        Some("latest") => state.store.latest_event_id().await?,
        Some(_) => return Err(invalid_after()),
    };
    let mut cursor = match headers.get("last-event-id") {
        None => start,
        Some(value) => value
            .to_str()
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|value| *value >= 0)
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "events.invalid_cursor",
                    "Last-Event-ID must be a nonnegative event sequence.",
                )
            })?,
    };
    let permit = state.streams.clone().try_acquire_owned().map_err(|_| {
        ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "events.limit",
            "Close an existing event stream and reconnect.",
        )
    })?;
    // Read before sending HTTP 200, so unavailable state is a normal JSON error.
    let initial = state.store.events_after(cursor).await?;
    let stream = async_stream::stream! {
        let _permit = permit;
        let mut pending = initial;
        loop {
            for event in pending {
                if state.shutdown.is_cancelled() { return; }
                cursor = event.id;
                let dto = EventDto::from(event);
                match Event::default().id(&dto.id).event(&dto.kind).json_data(&dto) {
                    Ok(event) => yield Ok::<_, Infallible>(event),
                    Err(_) => { tracing::error!(code = "events.encode_failed"); return; }
                }
            }
            tokio::select! {
                () = state.shutdown.cancelled() => break,
                () = tokio::time::sleep(EVENT_POLL) => {}
            }
            pending = match state.store.events_after(cursor).await {
                Ok(events) => events,
                Err(_) => {
                    tracing::error!(code = "events.read_failed");
                    yield Ok(Event::default().event("stream.error").data("{\"code\":\"state.unavailable\",\"message\":\"Reconnect using Last-Event-ID.\"}"));
                    break;
                }
            };
        }
    };
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(KEEP_ALIVE))
        .into_response())
}

/// Filters for `GET /activity`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivityQuery {
    /// Only this person or agent id.
    actor: Option<String>,
    /// Only events about this project.
    project_id: Option<String>,
    /// Only events of this kind, such as `agent.tool`.
    kind: Option<String>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
}

/// Committed activity, newest first, 50 per page, for activity screens.
async fn activity<S: StateStore>(
    State(state): State<ApiState<S>>,
    query: Result<Query<ActivityQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<super::contracts::ActivityPage>, ApiError> {
    let invalid = || {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "activity.invalid_query",
            "Use actor, project_id, kind and cursor only; cursor comes from the previous page.",
        )
    };
    let Query(query) = query.map_err(|_| invalid())?;
    let before = query
        .cursor
        .as_deref()
        .map(|value| {
            value
                .parse::<i64>()
                .ok()
                .filter(|id| *id > 0)
                .ok_or_else(invalid)
        })
        .transpose()?;
    if query
        .actor
        .as_ref()
        .is_some_and(|text| text.len() > MAX_FILTER_BYTES)
        || query
            .project_id
            .as_ref()
            .is_some_and(|text| text.len() > MAX_FILTER_BYTES)
        || query
            .kind
            .as_ref()
            .is_some_and(|text| text.len() > MAX_KIND_BYTES)
    {
        return Err(invalid());
    }
    let events = state
        .store
        .events_before(
            before,
            query.actor.as_deref(),
            query.project_id.as_deref(),
            query.kind.as_deref(),
        )
        .await?;
    let next_cursor = (events.len() == crate::application::EVENT_PAGE_SIZE)
        .then(|| events.last().map(|event| event.id.to_string()))
        .flatten();
    Ok(Json(super::contracts::ActivityPage {
        items: events.into_iter().map(EventDto::from).collect(),
        next_cursor,
    }))
}

async fn doctor<S: StateStore>(
    State(state): State<ApiState<S>>,
) -> Result<Json<super::contracts::DoctorReport>, ApiError> {
    let permit = state.doctor.clone().try_acquire_owned().map_err(|_| {
        ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "doctor.busy",
            "A keychain check is already running. Finish it and try again.",
        )
    })?;
    let report = crate::doctor::inspect(&state.store, "daemon", permit).await;
    let report = if let Some(engine) = state.engine {
        crate::doctor::accounts(report, &engine).await
    } else {
        report
    };
    Ok(Json(report))
}
