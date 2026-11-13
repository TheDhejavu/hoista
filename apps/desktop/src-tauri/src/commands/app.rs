use crate::error::Result;
use crate::files;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

const PREFERENCES: &str = "preferences.json";

/// What the webview needs before the screens start.
#[derive(Serialize)]
pub struct Startup {
    /// `darwin`, `linux` or `win32`, the names the screens use.
    platform: &'static str,
    /// This computer's account name, for the suggested profile name.
    user: String,
    /// The app's own preferences, as saved last time.
    preferences: Option<Value>,
    /// A screen to open on, from `HOISTA_START_AT`, to see first run again.
    start_at: Option<String>,
    /// Whether closing the window keeps Hoista running in the menu bar.
    tray: bool,
}

#[tauri::command]
pub fn startup(app: AppHandle) -> Startup {
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        _ => "linux",
    };
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    let preferences = preferences_file(&app)
        .and_then(|path| files::read(&path).ok().flatten())
        .and_then(|text| serde_json::from_str(&text).ok());
    let start_at = std::env::var("HOISTA_START_AT")
        .ok()
        .and_then(|given| screen_path(&given));
    Startup {
        platform,
        user,
        preferences,
        start_at,
        tray: crate::window::has_tray(),
    }
}

/// `welcome` or `/settings/general` as `/welcome`; anything else is ignored.
fn screen_path(given: &str) -> Option<String> {
    let path = given.trim().trim_start_matches(['#', '/']);
    let valid = path.starts_with(|c: char| c.is_ascii_lowercase())
        && path
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '/' | '-'));
    valid.then(|| format!("/{path}"))
}

#[tauri::command]
pub fn preferences_save(app: AppHandle, preferences: Value) -> Result<()> {
    let path = preferences_file(&app)
        .ok_or_else(|| crate::error::Error::new("app", "No folder for preferences."))?;
    files::write_private(&path, &serde_json::to_string_pretty(&preferences)?)
}

fn preferences_file(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(PREFERENCES))
}

/// The `hoist` CLI, if it's installed.
#[derive(Serialize)]
pub struct Cli {
    path: String,
    version: Option<String>,
}

/// Looks for `hoist` on the PATH and in the folders its installers use. An
/// app opened from the Dock gets a short PATH, so those are checked too.
#[tauri::command]
pub async fn find_cli() -> Option<Cli> {
    let path = cli_locations().into_iter().find(|path| path.is_file())?;
    let version = tokio::process::Command::new(&path)
        .arg("--version")
        .output()
        .await
        .ok()
        .and_then(|output| version_in(&String::from_utf8_lossy(&output.stdout)));
    Some(Cli {
        path: path.to_string_lossy().into_owned(),
        version,
    })
}

fn cli_locations() -> Vec<PathBuf> {
    let name = if cfg!(windows) { "hoist.exe" } else { "hoist" };
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let on_path = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default();
    let usual = [
        home.join(".local/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    on_path
        .into_iter()
        .chain(usual)
        .map(|folder| folder.join(name))
        .collect()
}

/// `hoist 0.1.0` as `0.1.0`.
fn version_in(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .find(|word| word.chars().next().is_some_and(|c| c.is_ascii_digit()) && word.contains('.'))
        .map(str::to_owned)
}

#[tauri::command]
pub fn quit(app: AppHandle) {
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_screen_path() {
        assert_eq!(screen_path("welcome").as_deref(), Some("/welcome"));
        assert_eq!(
            screen_path("#/settings/general").as_deref(),
            Some("/settings/general")
        );
        assert_eq!(screen_path("javascript:alert(1)"), None);
        assert_eq!(screen_path("../etc"), None);
    }

    #[test]
    fn reads_the_cli_version() {
        assert_eq!(version_in("hoist 0.1.0\n").as_deref(), Some("0.1.0"));
        assert_eq!(version_in("nothing here"), None);
    }
}
