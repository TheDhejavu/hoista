//! Local socket lifecycle; the state store must outlive the listener.

use crate::application::StateStore;
use hoista_system::paths::StatePaths;
use std::time::Duration;
#[cfg(unix)]
use std::{fs, os::unix::fs::FileTypeExt};
use tokio_util::sync::CancellationToken;

/// How long open connections get to finish at shutdown.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
/// A request this slow is noted in the log.
const SLOW_REQUEST: Duration = Duration::from_secs(2);
const MAX_LOGGED_PATH_CHARS: usize = 160;

/// Runs the local service until shutdown; never opens a TCP listener.
///
/// # Errors
/// Reports socket path, permission and serving failures.
pub async fn serve<S: StateStore>(
    paths: &StatePaths,
    store: S,
    shutdown: CancellationToken,
    engine: std::sync::Arc<crate::application::engine::Engine>,
) -> std::io::Result<()> {
    serve_with_lifecycle(paths, store, shutdown, engine, None).await
}

/// Serves the production daemon with optional owner restart coordination.
///
/// # Errors
/// Reports listener and graceful-drain failures.
pub async fn serve_with_lifecycle<S: StateStore>(
    paths: &StatePaths,
    store: S,
    shutdown: CancellationToken,
    engine: std::sync::Arc<crate::application::engine::Engine>,
    lifecycle: Option<std::sync::Arc<super::lifecycle::DaemonLifecycle>>,
) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let socket = paths.socket();
        match fs::symlink_metadata(&socket) {
            Ok(metadata) if metadata.file_type().is_socket() => fs::remove_file(&socket)?,
            Ok(_) => {
                return Err(std::io::Error::other(
                    "Refusing to replace a non-socket at hoistd.sock.",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    let listener = super::transport::LocalListener::bind(paths)?;
    tracing::info!(code = "daemon.ready");
    let (diagnostics, diagnostic_work) = super::diagnostics::router(engine.clone(), paths.clone());
    let (transfer, transfer_work) = super::transfer::router(engine.clone());
    let result = {
        let serving = axum::serve(
            listener,
            super::http::router_with_lifecycle(
                store,
                shutdown.clone(),
                Some(engine.clone()),
                lifecycle.clone(),
            )
            .merge(diagnostics)
            .merge(transfer)
            .merge(super::catalog::router(engine.clone()))
            .merge(super::backups::router(engine.clone()))
            .merge(super::pages::address_router(paths.clone()))
            .merge(super::link::router(engine.clone(), paths.clone()))
            .merge(super::mcp::router(engine))
            .route_layer(axum::middleware::from_fn(note_slow)),
        )
        .with_graceful_shutdown(shutdown.clone().cancelled_owned());
        let serving = std::future::IntoFuture::into_future(serving);
        tokio::pin!(serving);
        tokio::select! {
            result = &mut serving => result,
            () = shutdown.cancelled() => {
                // Restart drains accepted owner requests, including long provider mutations.
                if lifecycle.as_ref().is_some_and(|control| control.restart_requested()) {
                    serving.await
                } else {
                tokio::time::timeout(DRAIN_TIMEOUT, &mut serving).await
                    .unwrap_or_else(|_| Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "Local connections did not close within five seconds.")))
                }
            }
        }
    };
    diagnostic_work.drain().await;
    transfer_work.drain().await;
    result
}

/// Notes in the log any local request that took a while, by its path
/// (never the query, where anything sensitive could be), to find slow tabs.
async fn note_slow(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let route = request
        .uri()
        .path()
        .chars()
        .take(MAX_LOGGED_PATH_CHARS)
        .collect::<String>();
    let method = request.method().clone();
    let started = std::time::Instant::now();
    let response = next.run(request).await;
    let took = started.elapsed();
    if took >= SLOW_REQUEST {
        tracing::info!(
            code = "request.slow",
            method = method.as_str(),
            path = route.as_str(),
            status = response.status().as_u16(),
            ms = u64::try_from(took.as_millis()).unwrap_or(u64::MAX),
            "A request took a while"
        );
    }
    response
}
