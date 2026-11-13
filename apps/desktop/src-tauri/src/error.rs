use serde::Serialize;
use std::fmt;

/// What every command returns when it fails. The webview receives
/// `{ code, message }`, the same shape as the engine's own errors, so a
/// screen shows either one the same way.
#[derive(Debug, Clone, Serialize)]
pub struct Error {
    pub code: String,
    pub message: String,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::new("io", error.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::new("json", error.to_string())
    }
}

impl From<keyring::Error> for Error {
    fn from(error: keyring::Error) -> Self {
        Self::new(
            "keychain",
            format!("The system keychain refused it: {error}"),
        )
    }
}

impl From<tauri::Error> for Error {
    fn from(error: tauri::Error) -> Self {
        Self::new("app", error.to_string())
    }
}
