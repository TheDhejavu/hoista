//! Keeps history for as long as the person chose (`history_days`, a year by
//! default), pruning once a day: activity, finished operations and finished
//! webhook deliveries. Activity from the last 30 days is never removed; the
//! database refuses it. Engine logs rotate separately and keep 14 days.

use crate::{
    application::ports::catalog::{CatalogRepository, EngineError},
    store::SqliteStore,
};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// The first prune waits until start-up work has settled.
const FIRST_PRUNE: Duration = Duration::from_secs(5 * 60);
const PRUNE_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// Recent activity is always kept, whatever the setting says. The database
/// enforces it too.
pub const MIN_HISTORY_DAYS: u16 = 30;

/// Prunes shortly after start, then once a day, until `stop`.
pub async fn run(store: SqliteStore, stop: CancellationToken) {
    let mut wait = FIRST_PRUNE;
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            () = tokio::time::sleep(wait) => {}
        }
        wait = PRUNE_EVERY;
        match prune(&store).await {
            Ok(0) => {}
            Ok(removed) => tracing::info!(code = "history.pruned", removed),
            Err(error) => tracing::warn!(code = "history.prune_failed", %error),
        }
    }
}

/// Removes history older than the person's `history_days`.
///
/// # Errors
/// Fails when the database can't be read or written.
pub async fn prune(store: &SqliteStore) -> Result<u64, EngineError> {
    let days = store.settings().await?.history_days.max(MIN_HISTORY_DAYS);
    let before = (chrono::Utc::now() - chrono::Duration::days(i64::from(days)))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    store.prune_history(&before).await
}
