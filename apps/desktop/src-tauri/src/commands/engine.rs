use crate::engine::{Connection, Local, Reply, Request, Tails};
use crate::error::Result;
use crate::remotes::Remotes;
use serde::Serialize;
use tauri::State;
use tauri::ipc::Channel;

/// A request to the Hoista the screens show.
#[tauri::command]
pub async fn engine_request(request: Request, remotes: State<'_, Remotes>) -> Result<Reply> {
    remotes.connection().await.request(&request).await
}

/// A request to this computer's Hoista, whichever one the screens show.
#[tauri::command]
pub async fn local_request(request: Request, local: State<'_, Local>) -> Result<Reply> {
    local.request(&request).await
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TailMessage {
    Message { text: String },
    End,
}

/// Opens a live stream such as a log tail and returns its id for
/// `tail_stop`. Tails come from this computer only; while a remote is shown
/// the stream ends at once and the screen reads the logs instead.
#[tauri::command]
pub async fn tail_start(
    path: String,
    on_message: Channel<TailMessage>,
    remotes: State<'_, Remotes>,
    tails: State<'_, Tails>,
) -> Result<u32> {
    let id = tails.next_id();
    let Connection::Local(local) = remotes.connection().await else {
        let _closed = on_message.send(TailMessage::End);
        return Ok(id);
    };
    let tails_after = tails.inner().clone();
    let task = tauri::async_runtime::spawn(async move {
        let sink = on_message.clone();
        let _dropped = local
            .stream(&path, None, |text| {
                sink.send(TailMessage::Message { text }).is_ok()
            })
            .await;
        let _closed = on_message.send(TailMessage::End);
        tails_after.finished(id);
    });
    tails.insert(id, task);
    Ok(id)
}

#[tauri::command]
pub fn tail_stop(id: u32, tails: State<'_, Tails>) {
    tails.stop(id);
}
