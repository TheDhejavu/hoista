//! The app menu, the icon's menu and the right-click menu. Items that run
//! one of the app's commands send it to the main window as `cmd:<name>`;
//! the window runs it the same way as its keyboard shortcut.

use crate::error::Result;
use std::sync::Mutex;
use tauri::menu::{
    Menu, MenuBuilder, MenuEvent, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder,
};
use tauri::{AppHandle, Manager, Url, WebviewWindow, Wry};
use tauri_plugin_clipboard_manager::ClipboardExt;

const DOCS: &str = "https://docs.hoista.dev";
const REPORT_BUG: &str = "https://github.com/TheDhejavu/hoista/issues/new";

const OPEN_LINK: &str = "link:open:";
const COPY_LINK: &str = "link:copy:";

/// The main window's zoom level.
pub struct Zoom(Mutex<f64>);

impl Default for Zoom {
    fn default() -> Self {
        Self(Mutex::new(1.0))
    }
}

pub fn app_menu(app: &AppHandle) -> Result<Menu<Wry>> {
    let command =
        |name: &str, label: &str| MenuItemBuilder::with_id(format!("cmd:{name}"), label).build(app);
    let item = |id: &str, label: &str| MenuItemBuilder::with_id(id, label).build(app);
    let mac = cfg!(target_os = "macos");

    let mut menu = MenuBuilder::new(app);
    if mac {
        let hoista = SubmenuBuilder::new(app, "Hoista")
            .about(None)
            .separator()
            .item(&command("settings", "Settings…")?)
            .separator()
            .services()
            .separator()
            .hide()
            .hide_others()
            .show_all()
            .separator()
            .quit()
            .build()?;
        menu = menu.item(&hoista);
    }

    let mut file = SubmenuBuilder::new(app, "File")
        .item(&command("new-deploy", "Deploy Something")?)
        .separator()
        .item(&command("palette", "Search or Jump To…")?)
        .separator();
    file = if mac {
        file.close_window()
    } else {
        file.item(&command("settings", "Settings")?)
            .separator()
            .quit()
    };

    let edit = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let view = SubmenuBuilder::new(app, "View")
        .item(&command("refresh", "Refresh")?)
        .separator()
        .item(&item("zoom:reset", "Actual Size")?)
        .item(&item("zoom:in", "Zoom In")?)
        .item(&item("zoom:out", "Zoom Out")?)
        .separator()
        .fullscreen()
        .build()?;

    let go = SubmenuBuilder::new(app, "Go")
        .item(&command("go-projects", "Projects")?)
        .item(&command("go-deploys", "Deploys")?)
        .item(&command("go-agents", "Agents")?)
        .item(&command("go-accounts", "Accounts")?)
        .item(&command("go-billing", "Billing")?)
        .separator()
        .item(&command("back", "Back")?)
        .item(&command("forward", "Forward")?)
        .build()?;

    let window = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .build()?;

    let help = SubmenuBuilder::new(app, "Help")
        .item(&command("shortcuts", "Keyboard Shortcuts")?)
        .item(&command("help", "Get Help")?)
        .separator()
        .item(&item("open:docs", "Documentation")?)
        .item(&item("open:report-bug", "Report a Bug")?)
        .build()?;

    Ok(menu
        .item(&file.build()?)
        .item(&edit)
        .item(&view)
        .item(&go)
        .item(&window)
        .item(&help)
        .build()?)
}

/// What was right-clicked, as the page sees it.
#[derive(serde::Deserialize)]
pub struct Clicked {
    link: Option<String>,
    editable: bool,
    selected: bool,
}

/// Open or copy a link; cut, copy and paste in a field; copy a selection.
pub fn context_menu(window: &WebviewWindow, clicked: Clicked) -> Result<()> {
    let app = window.app_handle();
    let mut menu = MenuBuilder::new(app);
    let link = clicked
        .link
        .filter(|link| Url::parse(link).is_ok_and(|url| matches!(url.scheme(), "http" | "https")));
    if let Some(link) = &link {
        menu = menu
            .item(
                &MenuItemBuilder::with_id(format!("{OPEN_LINK}{link}"), "Open Link in Browser")
                    .build(app)?,
            )
            .item(&MenuItemBuilder::with_id(format!("{COPY_LINK}{link}"), "Copy Link").build(app)?);
    }
    if clicked.editable {
        if link.is_some() {
            menu = menu.separator();
        }
        menu = menu
            .undo()
            .redo()
            .separator()
            .cut()
            .copy()
            .paste()
            .separator()
            .select_all();
    } else if clicked.selected {
        if link.is_some() {
            menu = menu.separator();
        }
        menu = menu.item(&PredefinedMenuItem::copy(app, None)?);
    }
    let menu = menu.build()?;
    if !menu.items()?.is_empty() {
        window.popup_menu(&menu)?;
    }
    Ok(())
}

/// Every menu's clicks: the app menu, the icon's menu and the right-click menu.
pub fn on_click(app: &AppHandle, event: MenuEvent) {
    let id = event.id().as_ref();
    match id {
        "tray:open" => crate::window::show(app, None),
        "tray:settings" => crate::window::show(app, Some("cmd:settings")),
        "tray:quit" => app.exit(0),
        "zoom:reset" => zoom(app, |_| 1.0),
        "zoom:in" => zoom(app, |level| level + 0.1),
        "zoom:out" => zoom(app, |level| level - 0.1),
        "open:docs" => open(app, DOCS),
        "open:report-bug" => open(app, REPORT_BUG),
        _ if id.starts_with("cmd:") => crate::window::show(app, Some(id)),
        _ => {
            if let Some(link) = id.strip_prefix(OPEN_LINK) {
                open(app, link);
            } else if let Some(link) = id.strip_prefix(COPY_LINK) {
                let _ = app.clipboard().write_text(link);
            }
        }
    }
}

fn open(app: &AppHandle, link: &str) {
    if let Ok(url) = Url::parse(link) {
        crate::window::open_in_browser(app, &url);
    }
}

fn zoom(app: &AppHandle, change: impl FnOnce(f64) -> f64) {
    let Some(window) = app.get_webview_window(crate::window::MAIN) else {
        return;
    };
    let zoom = app.state::<Zoom>();
    let mut level = zoom.0.lock().expect("zoom lock");
    *level = (change(*level) * 10.0).round().clamp(5.0, 30.0) / 10.0;
    let _ = window.set_zoom(*level);
}
