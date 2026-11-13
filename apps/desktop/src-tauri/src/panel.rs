//! The small window under the menu bar icon. It shows the `#/menubar`
//! screen, and hides when it loses focus.

use crate::error::Result;
use tauri::{
    AppHandle, LogicalSize, Manager, PhysicalPosition, Rect, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};

pub const PANEL: &str = "panel";

const WIDTH: f64 = 372.0;
const MIN_HEIGHT: f64 = 220.0;
const MAX_HEIGHT: f64 = 720.0;
/// Gap between the menu bar and the panel, and from the screen's edge, in points.
const GAP: f64 = 6.0;
const MARGIN: f64 = 8.0;
/// The panel's height before its page says how tall it is.
const START_HEIGHT: f64 = 480.0;

pub fn create(app: &AppHandle) -> Result<WebviewWindow> {
    let links = app.clone();
    let builder = WebviewWindowBuilder::new(app, PANEL, WebviewUrl::App("index.html".into()))
        .title("Hoista")
        .inner_size(WIDTH, START_HEIGHT)
        .visible(false)
        .decorations(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .on_navigation(move |url| crate::window::stays_in_app(&links, url));
    // macOS draws its menu material behind the page; elsewhere it's a plain window.
    #[cfg(target_os = "macos")]
    let builder = builder.transparent(true).effects(
        tauri::window::EffectsBuilder::new()
            .effect(tauri::window::Effect::Menu)
            .state(tauri::window::EffectState::Active)
            .build(),
    );

    #[cfg(not(target_os = "macos"))]
    let builder = builder.background_color(tauri::window::Color(0xFC, 0xFC, 0xFC, 0xFF));

    let panel = builder.build()?;
    let hidden = panel.clone();
    panel.on_window_event(move |event| match event {
        WindowEvent::Focused(false) => {
            let _ = hidden.hide();
        }
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            let _ = hidden.hide();
        }
        _ => {}
    });
    Ok(panel)
}

/// Shows the panel under the icon, or hides it if it's showing.
pub fn toggle(app: &AppHandle, icon: Rect) {
    let Some(panel) = app.get_webview_window(PANEL) else {
        return;
    };
    if panel.is_visible().unwrap_or(false) {
        let _ = panel.hide();
        return;
    }
    place(&panel, icon);
    let _ = panel.show();
    let _ = panel.set_focus();
}

pub fn hide(app: &AppHandle) {
    if let Some(panel) = app.get_webview_window(PANEL) {
        let _ = panel.hide();
    }
}

/// Fits the panel to its content's height.
pub fn resize(app: &AppHandle, height: f64) {
    let Some(panel) = app.get_webview_window(PANEL) else {
        return;
    };
    if height.is_finite() {
        let height = height.clamp(MIN_HEIGHT, MAX_HEIGHT).round();
        let _ = panel.set_size(LogicalSize::new(WIDTH, height));
    }
}

/// Centres the panel under the icon, kept on screen. Linux tray icons often
/// don't say where they are; the panel then goes to the top right.
fn place(panel: &WebviewWindow, icon: Rect) {
    let Ok(Some(primary)) = panel.primary_monitor() else {
        return;
    };
    let scale = primary.scale_factor();
    let position = icon.position.to_physical::<f64>(scale);
    let size = icon.size.to_physical::<f64>(scale);
    let monitor = panel
        .monitor_from_point(position.x, position.y)
        .ok()
        .flatten()
        .unwrap_or(primary);
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let left = f64::from(area.position.x);
    let top = f64::from(area.position.y);
    let right = left + f64::from(area.size.width);
    let width = panel
        .outer_size()
        .map_or(WIDTH * scale, |size| f64::from(size.width));

    let (x, y) = if size.width > 0.0 {
        (
            position.x + size.width / 2.0 - width / 2.0,
            position.y + size.height + GAP * scale,
        )
    } else {
        (right - width - MARGIN * scale, top + GAP * scale)
    };
    let x = x.clamp(
        left + MARGIN * scale,
        (right - width - MARGIN * scale).max(left),
    );
    let _ = panel.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
}
