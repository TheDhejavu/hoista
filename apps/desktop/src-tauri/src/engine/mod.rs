//! Talking to a Hoista engine. Every screen talks to the one [`Connection`]
//! points at: this computer's `hoistd`, or one of the person's remotes.

mod connection;
mod events;
mod local;
mod sse;
mod tails;

pub use connection::Connection;
pub use events::follow_events;
pub use local::Local;
pub use sse::Splitter;
pub use tails::Tails;

use crate::error::{Error, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A request to an engine's API, as the webview sends it.
#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub body: Option<Value>,
}

impl Request {
    /// A known method and a path on the engine's API: no scheme, host or `..`.
    pub fn check(&self) -> Result<()> {
        const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];
        let path_ok =
            self.path.starts_with('/') && !self.path.starts_with("//") && !self.path.contains("..");
        if METHODS.contains(&self.method.as_str()) && path_ok {
            Ok(())
        } else {
            Err(Error::new(
                "request.invalid",
                "Not a request the engine takes.",
            ))
        }
    }

    pub fn get(path: impl Into<String>) -> Self {
        Self::new("GET", path, None)
    }

    pub fn new(method: &str, path: impl Into<String>, body: Option<Value>) -> Self {
        Self {
            method: method.to_owned(),
            path: path.into(),
            body,
        }
    }
}

/// An engine's whole reply. The webview reads it with the same code for
/// every engine; Rust code that needs the data calls [`Reply::data`].
#[derive(Debug, Serialize)]
pub struct Reply {
    pub status: u16,
    pub text: String,
}

/// HTTP statuses from here up are errors.
const FIRST_ERROR_STATUS: u16 = 400;

impl Reply {
    /// Whether the engine answered with an error.
    pub fn failed(&self) -> bool {
        self.status >= FIRST_ERROR_STATUS
    }

    /// The reply's JSON, or the engine's error as an [`Error`].
    pub fn data<T: DeserializeOwned>(self) -> Result<T> {
        if self.failed() {
            return Err(self.error());
        }
        let text = if self.text.trim().is_empty() {
            "null"
        } else {
            &self.text
        };
        Ok(serde_json::from_str(text)?)
    }

    /// The engine's error, from a failed reply.
    pub fn error(self) -> Error {
        #[derive(Deserialize)]
        struct Problem {
            code: Option<String>,
            message: Option<String>,
        }
        let problem = serde_json::from_str::<Problem>(&self.text).ok();
        let code = problem.as_ref().and_then(|p| p.code.clone());
        let message = problem.and_then(|p| p.message);
        Error::new(
            code.unwrap_or_else(|| format!("http_{}", self.status)),
            message.unwrap_or_else(|| "The Hoista engine returned an error.".to_owned()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::Request;

    #[test]
    fn takes_only_paths_on_the_engine() {
        assert!(Request::get("/projects?after=1").check().is_ok());
        assert!(Request::get("//evil.example.com").check().is_err());
        assert!(Request::get("/projects/../../etc").check().is_err());
        assert!(Request::get("https://evil.example.com").check().is_err());
        assert!(Request::new("TRACE", "/projects", None).check().is_err());
    }
}
