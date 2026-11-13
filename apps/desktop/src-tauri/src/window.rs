//! The main window. While the menu bar icon is there, closing the window
//! only hides it and Hoista keeps running.

use crate::error::Result;
use std::time::Duration;
use tauri::{
    AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_opener::OpenerExt;

pub const MAIN: &str = "main";

/// Where a screen or command is sent: `/projects/...`, or `cmd:palette`.
pub const NAVIGATE: &str = "navigate";

/// The page shows the window once it has painted, so it never flashes the
/// wrong colour; this shows it anyway in case the page never does.
const SHOW_ANYWAY_AFTER: Duration = Duration::from_secs(4);
/// The window's first size, and the smallest it can be made, in points.
const WIDTH: f64 = 1440.0;
const HEIGHT: f64 = 900.0;
const MIN_WIDTH: f64 = 960.0;
const MIN_HEIGHT: f64 = 640.0;

/// Whether this system gets the menu bar icon and panel.
pub fn has_tray() -> bool {
    cfg!(any(target_os = "macos", target_os = "linux"))
}

pub fn create(
    app: &AppHandle,
    menu: Option<tauri::menu::Menu<tauri::Wry>>,
) -> Result<WebviewWindow> {
    let links = app.clone();
    let popups = app.clone();
    let mut builder = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title("Hoista")
        .inner_size(WIDTH, HEIGHT)
        .min_inner_size(MIN_WIDTH, MIN_HEIGHT)
        .visible(false)
        .on_navigation(move |url| stays_in_app(&links, url))
        .on_new_window(move |url, _| {
            open_in_browser(&popups, &url);
            tauri::webview::NewWindowResponse::Deny
        });
    if let Some(menu) = menu {
        builder = builder.menu(menu);
    }

    #[cfg(target_os = "macos")]
    {
        builder = builder
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true)
            .traffic_light_position(tauri::LogicalPosition::new(18.0, 16.0));
    }
    let window = builder.build()?;

    let late = window.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SHOW_ANYWAY_AFTER).await;
        let _shown = late.show();
    });

    if has_tray() {
        let hidden = window.clone();
        window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _hidden = hidden.hide();
            }
        });
    }
    Ok(window)
}

/// Brings the main window forward and, if given, opens a screen or runs a command in it.
pub fn show(app: &AppHandle, target: Option<&str>) {
    crate::panel::hide(app);
    let Some(window) = app.get_webview_window(MAIN) else {
        return;
    };
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    if let Some(target) = target {
        let _ = window.emit_to(MAIN, NAVIGATE, target);
    }
}

/// A screen (`/projects/x`, `settings`) or a command (`cmd:palette`).
pub fn is_target(target: &str) -> bool {
    let screen = target.starts_with('/') && !target.starts_with("//") && !target.contains("..");
    let command = target.strip_prefix("cmd:").is_some_and(|name| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '-')
    });
    target == "settings" || screen || command
}

/// The app's own pages stay in the window; web links open in the browser.
pub fn stays_in_app(app: &AppHandle, url: &Url) -> bool {
    if is_app_page(app, url) {
        return true;
    }
    open_in_browser(app, url);
    false
}

/// The bundled page, or in development the dev server Tauri was told to load.
fn is_app_page(app: &AppHandle, url: &Url) -> bool {
    if url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost") {
        return true;
    }
    cfg!(debug_assertions)
        && app
            .config()
            .build
            .dev_url
            .as_ref()
            .is_some_and(|dev| dev.origin() == url.origin())
}

pub fn open_in_browser(app: &AppHandle, url: &Url) {
    if matches!(url.scheme(), "http" | "https") {
        let _ = app.opener().open_url(url.as_str(), None::<&str>);
    }
}
