use crate::error::Result;
use crate::menu::Clicked;
use crate::tray::IconState;
use tauri::{AppHandle, Manager, WebviewWindow};

/// Brings the main window forward, on a screen (`/agents`, `settings`) or
/// running a command (`cmd:palette`) if given.
#[tauri::command]
pub fn show_main(app: AppHandle, target: Option<String>) {
    let target = target.filter(|target| crate::window::is_target(target));
    crate::window::show(&app, target.as_deref());
}

/// Reloads every window, after switching to another Hoista.
#[tauri::command]
pub fn windows_reload(app: AppHandle) {
    for window in app.webview_windows().values() {
        let _ = window.reload();
    }
}

#[tauri::command]
pub fn panel_hide(app: AppHandle) {
    crate::panel::hide(&app);
}

#[tauri::command]
pub fn panel_resize(app: AppHandle, height: f64) {
    crate::panel::resize(&app, height);
}

#[tauri::command]
pub fn tray_show(app: AppHandle, state: IconState, waiting: u32) -> Result<()> {
    if crate::window::has_tray() {
        crate::tray::show(&app, state, waiting)?;
    }
    Ok(())
}

#[tauri::command]
pub fn context_menu(window: WebviewWindow, clicked: Clicked) -> Result<()> {
    crate::menu::context_menu(&window, clicked)
}
