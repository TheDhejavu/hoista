use crate::error::Result;
use crate::remotes::{Contents, Move, Remotes, View};
use tauri::State;

#[tauri::command]
pub async fn remotes_view(remotes: State<'_, Remotes>) -> Result<View> {
    remotes.view().await
}

#[tauri::command]
pub async fn remote_add(
    url: String,
    name: Option<String>,
    remotes: State<'_, Remotes>,
) -> Result<View> {
    remotes.add(&url, name.as_deref()).await
}

#[tauri::command]
pub async fn remote_rename(id: String, name: String, remotes: State<'_, Remotes>) -> Result<View> {
    remotes.rename(&id, &name).await
}

#[tauri::command]
pub async fn remote_forget(id: String, remotes: State<'_, Remotes>) -> Result<View> {
    remotes.forget(&id).await
}

#[tauri::command]
pub async fn remote_sign_in(id: String, remotes: State<'_, Remotes>) -> Result<View> {
    remotes.sign_in(&id).await
}

#[tauri::command]
pub async fn remote_sign_out(id: String, remotes: State<'_, Remotes>) -> Result<View> {
    remotes.sign_out(&id).await
}

#[tauri::command]
pub async fn remote_show(showing: String, remotes: State<'_, Remotes>) -> Result<View> {
    remotes.show(&showing).await
}

#[tauri::command]
pub async fn remote_contents(id: String, remotes: State<'_, Remotes>) -> Result<Contents> {
    remotes.contents(&id).await
}

#[tauri::command]
pub async fn remote_push(id: String, remotes: State<'_, Remotes>) -> Result<Move> {
    remotes.push(&id).await
}

/// Whether a Hoista answers at `url`, such as one just set up on a server.
#[tauri::command]
pub async fn remote_answers(url: String, remotes: State<'_, Remotes>) -> Result<bool> {
    Ok(remotes.answers_at(&url).await)
}
