//! Bounded JSON client over local IPC, or over HTTPS to Hoista on the person's
//! own server once this computer is signed in there. Neither follows redirects.

use super::contracts::{DoctorReport, ErrorDto, ProfileDto};
use axum::http::{Method, Request};
use hoista_system::paths::StatePaths;
use http_body_util::{BodyExt, Full, Limited};
use hyper_util::rt::TokioIo;
use std::time::Duration;

/// The largest answer the daemon or a server may give.
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const SOCKET_TIMEOUT: Duration = Duration::from_secs(5);
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// For calls that build and deploy.
const LONG_CALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// Moving everything to or from a server.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);

/// Local transport or API failure.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The daemon could not be reached or returned invalid data.
    #[error(
        "The daemon request could not be completed. Its outcome may be unknown. Check hoist daemon status for this state directory and inspect the operation before retrying a change."
    )]
    Transport,
    /// The API rejected the request.
    #[error("{0}")]
    Rejected(String),
}

/// Thin daemon client for CLI and stdio MCP commands.
pub struct LocalClient {
    pub(super) paths: StatePaths,
    pub(super) server: Option<Server>,
}

/// Hoista on the person's server, reached with this computer's key.
pub(super) struct Server {
    /// `https://host/api`.
    api: String,
    key: crate::domain::Secret,
    http: reqwest::Client,
}

/// A reply's JSON body; an empty one (204 No Content) reads as `null`.
fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, ClientError> {
    if bytes.is_empty() {
        return serde_json::from_value(serde_json::Value::Null).map_err(|_| ClientError::Transport);
    }
    serde_json::from_slice(bytes).map_err(|_| ClientError::Transport)
}

/// Checks a server address: `https://host`, or plain HTTP to this computer
/// for tests. Returns it without a trailing slash.
///
/// # Errors
/// Rejects anything else.
pub fn server_origin(url: &str) -> Result<String, ClientError> {
    let parsed = url::Url::parse(url.trim())
        .map_err(|_| ClientError::Rejected("That isn't a web address.".into()))?;
    let loopback = matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    let fine = (parsed.scheme() == "https" || (parsed.scheme() == "http" && loopback))
        && parsed.host_str().is_some()
        && matches!(parsed.path(), "" | "/")
        && parsed.query().is_none()
        && parsed.username().is_empty()
        && parsed.password().is_none();
    if !fine {
        return Err(ClientError::Rejected(
            "Use your Hoista's address, like https://hoista-production.up.railway.app.".into(),
        ));
    }
    Ok(parsed.as_str().trim_end_matches('/').to_owned())
}

impl LocalClient {
    /// Selects the owner's local endpoint.
    ///
    /// # Errors
    /// Kept fallible for endpoint validation by future platform adapters.
    pub fn new(paths: &StatePaths) -> Result<Self, ClientError> {
        Ok(Self {
            paths: paths.clone(),
            server: None,
        })
    }

    /// Talks to Hoista on the person's server at `origin` instead, with the
    /// key this computer got when the person allowed it there.
    ///
    /// # Errors
    /// Rejects an address that isn't the server's own.
    pub fn on_server(
        paths: &StatePaths,
        origin: &str,
        key: crate::domain::Secret,
    ) -> Result<Self, ClientError> {
        let origin = server_origin(origin)?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent(concat!("hoist/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| ClientError::Transport)?;
        Ok(Self {
            paths: paths.clone(),
            server: Some(Server {
                api: format!("{origin}/api"),
                key,
                http,
            }),
        })
    }

    /// Whether this client talks to a server rather than this computer.
    #[must_use]
    pub const fn is_on_server(&self) -> bool {
        self.server.is_some()
    }

    /// Sends a JSON request to `hoistd`, or to the server when signed in there.
    ///
    /// # Errors
    /// Returns transport, timeout, response-size or API errors.
    pub async fn request<T: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<T, ClientError> {
        self.authenticated(method, path, body, None).await
    }

    /// Sends a request with an optional agent bearer credential over owner-restricted IPC.
    /// A credential is refused on a server: agents connect to it directly.
    ///
    /// # Errors
    /// Returns transport or server rejection errors.
    pub async fn authenticated<T: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&serde_json::Value>,
        credential: Option<&str>,
    ) -> Result<T, ClientError> {
        // An agent's tool call can be a whole deploy (`modal deploy`, a
        // first build at a provider) or wait_for_deploy's two minutes; giving
        // up sooner tells the agent it failed while it's still going.
        let deadline = if path == "/mcp" {
            LONG_CALL_TIMEOUT
        } else {
            CALL_TIMEOUT
        };
        self.exchange(method, path, body, credential, deadline)
            .await
    }

    async fn exchange<T: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&serde_json::Value>,
        credential: Option<&str>,
        deadline: Duration,
    ) -> Result<T, ClientError> {
        if !path.starts_with('/') || path.starts_with("//") {
            return Err(ClientError::Transport);
        }
        if let Some(server) = &self.server {
            if credential.is_some() {
                return Err(ClientError::Rejected(
                    "Agents connect to your server at its own address, not through hoist.".into(),
                ));
            }
            return server.exchange(method, path, body, deadline).await;
        }
        let operation = async {
            let stream =
                tokio::time::timeout(SOCKET_TIMEOUT, super::transport::connect(&self.paths))
                    .await
                    .map_err(|_| ClientError::Transport)?
                    .map_err(|_| ClientError::Transport)?;
            let (mut sender, connection) =
                hyper::client::conn::http1::handshake(TokioIo::new(stream))
                    .await
                    .map_err(|_| ClientError::Transport)?;
            tokio::pin!(connection);
            let result = async {
                let bytes = match body {
                    Some(body) => serde_json::to_vec(body).map_err(|_| ClientError::Transport)?,
                    None => Vec::new(),
                };
                let mut request = Request::builder()
                    .method(method)
                    .uri(path)
                    .header("host", "localhost")
                    .header("content-type", "application/json");
                if let Some(credential) = credential {
                    request = request.header("authorization", format!("Bearer {credential}"));
                }
                let request = request
                    .body(Full::new(axum::body::Bytes::from(bytes)))
                    .map_err(|_| ClientError::Transport)?;
                let response = sender
                    .send_request(request)
                    .await
                    .map_err(|_| ClientError::Transport)?;
                let status = response.status();
                let bytes = Limited::new(response.into_body(), MAX_RESPONSE_BYTES)
                    .collect()
                    .await
                    .map_err(|_| ClientError::Transport)?
                    .to_bytes();
                if !status.is_success() {
                    let error: ErrorDto =
                        serde_json::from_slice(&bytes).map_err(|_| ClientError::Transport)?;
                    return Err(ClientError::Rejected(error.explanation()));
                }
                decode(&bytes)
            };
            let result = tokio::time::timeout(deadline, result);
            tokio::pin!(result);
            tokio::select! {
                biased;
                result = &mut result => result.map_err(|_| ClientError::Transport)?,
                _ = &mut connection => {
                    // A graceful server close may finish the connection driver just
                    // before the buffered response is polled. Let the request consume
                    // that response (or its closed-channel error), under the same deadline.
                    result.await.map_err(|_| ClientError::Transport)?
                },
            }
        };
        operation.await
    }

    /// Checks local daemon readiness without touching credentials or provider accounts.
    ///
    /// # Errors
    /// Reports transport failure or an incompatible readiness response.
    pub async fn health(&self) -> Result<super::contracts::HealthDto, ClientError> {
        self.request(Method::GET, "/health", None).await
    }

    /// Reads a JSON resource from the local daemon.
    ///
    /// # Errors
    /// Reports daemon transport or request rejection.
    pub async fn get_json(&self, path: &str) -> Result<serde_json::Value, ClientError> {
        self.request(Method::GET, path, None).await
    }

    /// Sends a JSON command to the local daemon.
    ///
    /// # Errors
    /// Reports daemon transport or command rejection.
    pub async fn post_json(
        &self,
        path: &str,
        value: &serde_json::Value,
    ) -> Result<serde_json::Value, ClientError> {
        self.request(Method::POST, path, Some(value)).await
    }

    /// Updates a JSON resource through the owner-local daemon API.
    ///
    /// # Errors
    /// Reports transport failures or rejected updates.
    pub async fn put_json(
        &self,
        path: &str,
        value: &serde_json::Value,
    ) -> Result<serde_json::Value, ClientError> {
        self.request(Method::PUT, path, Some(value)).await
    }

    /// Applies a partial owner-local settings update through the daemon API.
    ///
    /// # Errors
    /// Reports transport failures or rejected settings updates.
    pub async fn patch_json(
        &self,
        path: &str,
        value: &serde_json::Value,
    ) -> Result<serde_json::Value, ClientError> {
        self.request(Method::PATCH, path, Some(value)).await
    }

    /// Removes an owner-local resource through the daemon API.
    ///
    /// # Errors
    /// Reports transport failures or rejected deletion requests.
    pub async fn delete_json(&self, path: &str) -> Result<serde_json::Value, ClientError> {
        self.request(Method::DELETE, path, None).await
    }

    /// Sends an explicit confirmation body for a reviewed deletion.
    ///
    /// # Errors
    /// Reports transport errors, stale review or rejected deletion.
    pub async fn delete_json_with_body(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ClientError> {
        self.exchange(Method::DELETE, path, Some(body), None, TRANSFER_TIMEOUT)
            .await
    }

    /// Waits up to two hours for an explicit owner export; timeout never authorizes replay.
    ///
    /// # Errors
    /// Reports transport errors or an incomplete operation whose status must be inspected.
    pub async fn post_json_long_running(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ClientError> {
        self.exchange(Method::POST, path, Some(body), None, TRANSFER_TIMEOUT)
            .await
    }

    /// Reads a configured profile.
    ///
    /// # Errors
    /// Reports local transport or profile absence.
    pub async fn profile(&self) -> Result<ProfileDto, ClientError> {
        self.request(Method::GET, "/profile", None).await
    }

    /// Saves a profile through the daemon.
    ///
    /// # Errors
    /// Fails when `hoistd` can't be reached, refuses the request, or can't save.
    pub async fn save_profile(&self, profile: &ProfileDto) -> Result<ProfileDto, ClientError> {
        let body = serde_json::to_value(profile).map_err(|_| ClientError::Transport)?;
        self.request(Method::PUT, "/profile", Some(&body)).await
    }

    /// Runs diagnostics through the current owner.
    ///
    /// # Errors
    /// Reports local transport or API failures.
    pub async fn doctor(&self) -> Result<DoctorReport, ClientError> {
        self.request(Method::GET, "/doctor", None).await
    }
}

impl Server {
    async fn exchange<T: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&serde_json::Value>,
        deadline: Duration,
    ) -> Result<T, ClientError> {
        let method = reqwest::Method::from_bytes(method.as_str().as_bytes())
            .map_err(|_| ClientError::Transport)?;
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.api))
            .bearer_auth(self.key.expose())
            .timeout(deadline);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.map_err(|_| unreachable())?;
        let status = response.status();
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err(ClientError::Transport);
        }
        let bytes = response.bytes().await.map_err(|_| unreachable())?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(ClientError::Transport);
        }
        if !status.is_success() {
            return Err(ClientError::Rejected(
                serde_json::from_slice::<ErrorDto>(&bytes).map_or_else(
                    |_| {
                        format!("Your Hoista's server answered {status}. Nothing here was changed.")
                    },
                    |error| error.explanation(),
                ),
            ));
        }
        decode(&bytes)
    }
}

fn unreachable() -> ClientError {
    ClientError::Rejected("Couldn't reach your Hoista's server, or it stopped answering. If you were changing something, check it on the server before trying again.".into())
}
