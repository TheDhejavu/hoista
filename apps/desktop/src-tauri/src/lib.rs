//! Hoista Desktop. The screens are React, in the webview; this side owns
//! everything around them: which Hoista they talk to (this computer's
//! `hoistd`, or one of your remotes), the windows, the menu bar icon and
//! menus, and what the app keeps on disk and in the keychain.

mod commands;
mod engine;
mod error;
mod files;
mod loopback;
mod menu;
mod panel;
mod paths;
mod remotes;
mod tray;
mod window;

use engine::{Local, Tails};
use remotes::Remotes;
use tauri::{App, AppHandle, Manager, RunEvent};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_window_state::StateFlags;

/// The longest this computer's name is shown, in characters.
const MAX_NAME_CHARS: usize = 60;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        // The main window reopens where it was; the panel always goes under the icon.
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(StateFlags::all() - StateFlags::VISIBLE)
                .with_denylist(&[panel::PANEL])
                .build(),
        )
        .manage(Tails::default())
        .manage(menu::Zoom::default())
        .on_menu_event(menu::on_click)
        .setup(|app| Ok(setup(app)?))
        .invoke_handler(tauri::generate_handler![
            commands::app::startup,
            commands::app::preferences_save,
            commands::app::find_cli,
            commands::app::quit,
            commands::engine::engine_request,
            commands::engine::local_request,
            commands::engine::tail_start,
            commands::engine::tail_stop,
            commands::remotes::remotes_view,
            commands::remotes::remote_add,
            commands::remotes::remote_rename,
            commands::remotes::remote_forget,
            commands::remotes::remote_sign_in,
            commands::remotes::remote_sign_out,
            commands::remotes::remote_show,
            commands::remotes::remote_contents,
            commands::remotes::remote_push,
            commands::remotes::remote_answers,
            commands::windows::show_main,
            commands::windows::windows_reload,
            commands::windows::panel_hide,
            commands::windows::panel_resize,
            commands::windows::tray_show,
            commands::windows::context_menu,
        ])
        .build(tauri::generate_context!())
        .expect("Hoista couldn't start")
        .run(on_event);
}

fn setup(app: &mut App) -> error::Result<()> {
    let handle = app.handle().clone();
    let local = Local::new(paths::engine_socket());
    let (remotes, shown) = Remotes::new(remotes::Setup {
        local: local.clone(),
        state_folder: paths::state_folder(),
        config_folder: app.path().app_config_dir()?,
        moves_folder: app.path().app_data_dir()?.join("moves"),
        computer: computer_name(),
        open_page: Box::new(move |link| {
            let _ = handle.opener().open_url(link, None::<&str>);
        }),
    });
    app.manage(local);
    app.manage(remotes);
    tauri::async_runtime::spawn(engine::follow_events(app.handle().clone(), shown));

    let handle = app.handle();
    let menu = menu::app_menu(handle)?;
    // On macOS the menu belongs to the app; elsewhere only the main window has one.
    if cfg!(target_os = "macos") {
        app.set_menu(menu)?;
        window::create(handle, None)?;
    } else {
        window::create(handle, Some(menu))?;
    }
    if window::has_tray() {
        panel::create(handle)?;
        tray::create(handle)?;
    }
    Ok(())
}

fn on_event(app: &AppHandle, event: RunEvent) {
    // macOS: clicking the Dock icon brings the window back.
    #[cfg(target_os = "macos")]
    if let RunEvent::Reopen { .. } = event {
        window::show(app, None);
    }
    let _ = (app, event);
}

/// This computer's name, as remotes list it.
fn computer_name() -> String {
    let name = gethostname::gethostname().to_string_lossy().into_owned();
    let name = name.trim().trim_end_matches(".local");
    let name = if name.is_empty() { "Hoista app" } else { name };
    name.chars().take(MAX_NAME_CHARS).collect()
}
