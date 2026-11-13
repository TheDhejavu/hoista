//! The menu bar icon. Its picture follows Hoista's state, which the main
//! window works out from the engine and passes here.

use crate::error::Result;
use serde::Deserialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::async_runtime::JoinHandle;
use tauri::image::Image;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

const TRAY: &str = "hoista";
const FRAME_EVERY: Duration = Duration::from_millis(120);

/// What the icon shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IconState {
    Normal,
    /// Something waits for the person: an amber dot.
    Needs,
    /// A deploy is running: the mark turns.
    Deploying,
    /// Stop everything is on: agents can't reach Hoista.
    Stopped,
}

/// One picture in black and white ink, 32 px for the menu bar's 16 pt.
struct Icon {
    black: &'static [u8],
    white: &'static [u8],
}

macro_rules! icon {
    ($name:literal) => {
        Icon {
            black: include_bytes!(concat!("../icons/tray/", $name, "-black.png")),
            white: include_bytes!(concat!("../icons/tray/", $name, "-white.png")),
        }
    };
}

const NORMAL: Icon = icon!("normal");
const NEEDS: Icon = icon!("needs");
const STOPPED: Icon = icon!("stopped");
const DEPLOYING: [Icon; 8] = [
    icon!("deploying-0"),
    icon!("deploying-1"),
    icon!("deploying-2"),
    icon!("deploying-3"),
    icon!("deploying-4"),
    icon!("deploying-5"),
    icon!("deploying-6"),
    icon!("deploying-7"),
];

/// The turning icon while a deploy runs.
#[derive(Default)]
pub struct Animation(Mutex<Option<JoinHandle<()>>>);

pub fn create(app: &AppHandle) -> Result<()> {
    let menu = MenuBuilder::new(app)
        .item(&MenuItemBuilder::with_id("tray:open", "Open Hoista").build(app)?)
        .item(&MenuItemBuilder::with_id("tray:settings", "Settings…").build(app)?)
        .separator()
        .item(&MenuItemBuilder::with_id("tray:quit", "Quit Hoista").build(app)?)
        .build()?;
    TrayIconBuilder::with_id(TRAY)
        .icon(picture(&NORMAL, true)?)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Hoista")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } = event
            {
                crate::panel::toggle(tray.app_handle(), rect);
            }
        })
        .build(app)?;
    app.manage(Animation::default());
    Ok(())
}

/// Shows `state` on the icon, and how many things wait on the Dock badge.
pub fn show(app: &AppHandle, state: IconState, waiting: u32) -> Result<()> {
    let tooltip = match waiting {
        0 => "Hoista".to_owned(),
        n => format!("Hoista · {n} waiting for you"),
    };
    if let Some(tray) = app.tray_by_id(TRAY) {
        tray.set_tooltip(Some(&tooltip))?;
    }
    if let Some(window) = app.get_webview_window(crate::window::MAIN) {
        let _ = window.set_badge_count((waiting > 0).then_some(i64::from(waiting)));
    }

    let animation = app.state::<Animation>();
    let mut animation = animation.0.lock().expect("animation lock");
    if let Some(running) = animation.take() {
        running.abort();
    }
    match state {
        IconState::Deploying => *animation = Some(turn(app.clone())),
        IconState::Normal => set(app, &NORMAL, true)?,
        IconState::Stopped => set(app, &STOPPED, true)?,
        // The amber dot can't be tinted by macOS, so it isn't a template.
        IconState::Needs => set(app, &NEEDS, false)?,
    }
    Ok(())
}

fn turn(app: AppHandle) -> JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        for frame in DEPLOYING.iter().cycle() {
            if set(&app, frame, true).is_err() {
                return;
            }
            tokio::time::sleep(FRAME_EVERY).await;
        }
    })
}

fn set(app: &AppHandle, icon: &Icon, template: bool) -> Result<()> {
    let Some(tray) = app.tray_by_id(TRAY) else {
        return Ok(());
    };
    let dark_bar = app
        .get_webview_window(crate::window::MAIN)
        .and_then(|window| window.theme().ok())
        .is_some_and(|theme| theme == tauri::Theme::Dark);
    tray.set_icon(Some(picture(icon, template || !dark_bar)?))?;
    tray.set_icon_as_template(template && cfg!(target_os = "macos"))?;
    Ok(())
}

/// macOS tints black template images to match the menu bar. Linux panels
/// are dark, so they get white ink.
fn picture(icon: &Icon, black_on_mac: bool) -> Result<Image<'static>> {
    let bytes = if cfg!(target_os = "macos") && black_on_mac {
        icon.black
    } else {
        icon.white
    };
    Ok(Image::from_bytes(bytes)?)
}
