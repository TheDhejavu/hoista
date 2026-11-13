use super::{Connection, sse};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::watch;

/// The event every window listens to, with one server-sent message as text.
pub const ENGINE_EVENT: &str = "engine-event";

const RETRY_AFTER: Duration = Duration::from_secs(2);

/// Follows the live events of whichever Hoista is shown, for as long as the
/// app runs, and passes each one to every window. Switching to another
/// Hoista drops the old stream and starts on the new one's newest event.
pub async fn follow_events(app: AppHandle, mut shown: watch::Receiver<Connection>) {
    let mut last_id: Option<String> = None;
    loop {
        let connection = shown.borrow_and_update().clone();
        let follow = follow_once(&app, &connection, &mut last_id);
        tokio::select! {
            () = follow => {}
            changed = shown.changed() => {
                if changed.is_err() {
                    return;
                }
                last_id = None;
                continue;
            }
        }
        tokio::select! {
            () = tokio::time::sleep(RETRY_AFTER) => {}
            changed = shown.changed() => {
                if changed.is_err() {
                    return;
                }
                last_id = None;
            }
        }
    }
}

/// Follows one stream until it drops. A first connection starts at the
/// newest event; a reconnection resumes after the last one seen.
async fn follow_once(app: &AppHandle, connection: &Connection, last_id: &mut Option<String>) {
    let resume = last_id.clone();
    let mut pass_on = |message: String| {
        if let Some(id) = sse::field(&message, "id") {
            *last_id = Some(id.to_owned());
        }
        app.emit(ENGINE_EVENT, message).is_ok()
    };
    // A dropped stream is retried by the caller; there's nothing to report.
    let _dropped = match connection {
        Connection::Local(local) => {
            let path = if resume.is_some() {
                "/events"
            } else {
                "/events?after=latest"
            };
            local.stream(path, resume.as_deref(), &mut pass_on).await
        }
        Connection::Remote(session) => session.events(&mut pass_on).await,
    };
}
