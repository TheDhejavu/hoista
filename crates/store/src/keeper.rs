//! The credential keeper: a separate process that holds provider tokens so the
//! process agents talk to never does. It adds a token only to requests for
//! the fixed provider APIs, never to a redirect, and never hands one back.
//!
//! Messages are length-prefixed JSON over a Unix socket.

use super::{
    onepassword::{self, ReferencedVault},
    process::{self, ProcessError},
    token_store::{BuiltIn, StoredVault, TokenStore, provider_token},
    tool_paths,
};
use crate::{
    Provider,
    application::ports::{catalog::EngineError, keeper::CredentialKeeper, vault::TokenVault},
    domain::{Secret, model::VaultPlace},
    providers::{self, SignIn},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

pub use crate::application::ports::keeper::KeptResponse;

/// The largest message either side sends: a provider response, base64'd.
const MAX_FRAME_BYTES: usize = 24 * 1024 * 1024;
/// The largest provider response passed back.
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const PROVIDER_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PROVIDER_TIMEOUT: Duration = Duration::from_secs(120);
/// A provider's command-line tool can upload a whole build.
const CLI_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const KEEPER_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// The longest a call can take: a CLI run, plus a minute to spare.
const REPLY_TIMEOUT: Duration = Duration::from_secs(CLI_TIMEOUT.as_secs() + 60);
/// Bytes of the value's SHA-256 that name a held value.
const HELD_DIGEST_BYTES: usize = 16;

/// What the main process asks for.
#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Request {
    /// Whether an entry exists.
    Has { key: String },
    /// An entry that isn't a provider token.
    Read { key: String },
    /// Sets an entry.
    Write { key: String, value: String },
    /// Removes an entry.
    Delete { key: String },
    /// Holds a 1Password reference (or token) under a pending key, for checking it on connect.
    Hold { value: String },
    /// A provider request; the keeper signs it in with the token named by `key`.
    Http {
        key: String,
        method: String,
        url: String,
        headers: Vec<(String, String)>,
        body: String,
        /// Sent over HTTP/2, as gRPC needs.
        http2: bool,
    },
    /// A provider's own command-line tool (see `ProviderCli`) with the token named
    /// by `key`.
    ProviderCli {
        provider: Provider,
        key: String,
        args: Vec<String>,
        dir: Option<String>,
        stdin: Option<String>,
    },
}

/// What the keeper answers.
#[derive(Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
enum Response {
    Done,
    Found {
        found: bool,
    },
    Value {
        value: String,
    },
    Http {
        status: u16,
        headers: Vec<(String, String)>,
        body: String,
    },
    Ran {
        success: bool,
    },
    Refused {
        code: String,
        message: String,
    },
}

fn refused(code: &str, message: &str) -> Response {
    Response::Refused {
        code: code.into(),
        message: message.into(),
    }
}

/// The keeper's side.
pub struct Keeper {
    /// The sealed file, with provider tokens through its token store: the same
    /// vault a computer uses, in a different place.
    vault: StoredVault,
    /// Reads 1Password references a person connected an account with.
    references: ReferencedVault,
    http: reqwest::Client,
    /// The main process's user; anyone else is refused.
    allowed_uid: Option<u32>,
    /// In tests, a provider on 127.0.0.1 over plain HTTP stands in for this one.
    #[cfg(test)]
    local_provider: Option<Provider>,
}

impl Keeper {
    /// A keeper for `vault`, answering only `allowed_uid` when given.
    ///
    /// # Errors
    /// Reports a TLS client that can't be built.
    pub fn new(vault: Arc<dyn TokenVault>, allowed_uid: Option<u32>) -> Result<Self, EngineError> {
        let http = reqwest::Client::builder()
            .https_only(!cfg!(test))
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(PROVIDER_CONNECT_TIMEOUT)
            .timeout(PROVIDER_TIMEOUT)
            .user_agent(concat!("hoista/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| EngineError::Store)?;
        Ok(Self {
            references: ReferencedVault::new(vault.clone()),
            vault: StoredVault::new(vault, VaultPlace::SealedFile, Arc::new(BuiltIn)),
            http,
            allowed_uid,
            #[cfg(test)]
            local_provider: None,
        })
    }

    /// Keeps provider tokens in `store` instead of the sealed file itself.
    #[must_use]
    pub fn with_store(mut self, store: Arc<dyn TokenStore>) -> Self {
        self.vault = self.vault.with_store(store);
        self
    }

    /// Answers on `socket` until `stop`.
    ///
    /// # Errors
    /// Reports a socket that can't be opened.
    #[cfg(unix)]
    pub async fn serve(
        self: Arc<Self>,
        socket: PathBuf,
        stop: tokio_util::sync::CancellationToken,
    ) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::remove_file(&socket);
        let listener = tokio::net::UnixListener::bind(&socket)?;
        // Who may connect is checked per connection, by user id.
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o666))?;
        loop {
            let (stream, _) = tokio::select! {
                () = stop.cancelled() => return Ok(()),
                accepted = listener.accept() => accepted?,
            };
            let peer = stream.peer_cred().ok().map(|cred| cred.uid());
            if self.allowed_uid.is_some() && peer != self.allowed_uid {
                tracing::warn!(code = "keeper.refused_peer", peer);
                continue;
            }
            let keeper = self.clone();
            tokio::spawn(async move {
                let _ = keeper.connection(stream).await;
            });
        }
    }

    #[cfg(unix)]
    async fn connection(&self, mut stream: tokio::net::UnixStream) -> std::io::Result<()> {
        while let Some(bytes) = read_frame(&mut stream).await? {
            let response = match serde_json::from_slice::<Request>(&bytes) {
                Ok(request) => self.answer(request).await,
                Err(_) => refused("keeper.bad_request", "The keeper didn't understand that."),
            };
            let reply = serde_json::to_vec(&response).map_err(std::io::Error::other)?;
            write_frame(&mut stream, &reply).await?;
        }
        Ok(())
    }

    async fn answer(&self, request: Request) -> Response {
        match request {
            Request::Has { key } => match self.vault.has(&key).await {
                Ok(found) => Response::Found { found },
                Err(error) => failed(&error),
            },
            Request::Read { key } if provider_token(&key) => refused(
                "keeper.provider_token",
                "Provider tokens stay with the credential keeper.",
            ),
            Request::Read { key } => match self.vault.read(&key).await {
                Ok(value) => Response::Value {
                    value: value.expose().to_owned(),
                },
                Err(error) => failed(&error),
            },
            Request::Write { key, value } => {
                into_done(self.vault.write(&key, Secret::new(value)).await)
            }
            Request::Delete { key } => into_done(self.vault.delete(&key).await),
            Request::Hold { value } => {
                use sha2::Digest as _;
                let digest: String = sha2::Sha256::digest(value.as_bytes())
                    .iter()
                    .take(HELD_DIGEST_BYTES)
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
                let key = format!("pending/{digest}");
                match self.vault.write(&key, Secret::new(value)).await {
                    Ok(()) => Response::Value { value: key },
                    Err(error) => failed(&error),
                }
            }
            Request::Http {
                key,
                method,
                url,
                headers,
                body,
                http2,
            } => self.http(&key, &method, &url, headers, &body, http2).await,
            Request::ProviderCli {
                provider,
                key,
                args,
                dir,
                stdin,
            } => self.provider_cli(provider, &key, args, dir, stdin).await,
        }
    }

    async fn token(&self, key: &str) -> Result<Secret, Response> {
        if !provider_token(key) {
            return Err(refused(
                "keeper.not_provider_token",
                "That isn't a provider account.",
            ));
        }
        // Resolves a 1Password reference here too, so its value stays here.
        let token = self.vault.read(key).await.map_err(|error| failed(&error))?;
        self.references
            .resolve(&token)
            .await
            .map_err(|error| failed(&error))
    }

    /// How the provider at `url` takes its token, if `url` is one of the providers'
    /// own APIs over HTTPS. This comes from the provider definitions built into
    /// Hoista, never from the request.
    fn sign_in_for(&self, url: &reqwest::Url) -> Option<&'static SignIn> {
        let name = url.host_str()?;
        #[cfg(test)]
        if let Some(provider) = self.local_provider
            && name == "127.0.0.1"
        {
            return Some(providers::sign_in(provider));
        }
        if url.scheme() != "https" {
            return None;
        }
        providers::sign_in_for_host(name)
    }

    async fn http(
        &self,
        key: &str,
        method: &str,
        url: &str,
        headers: Vec<(String, String)>,
        body: &str,
        http2: bool,
    ) -> Response {
        let Ok(parsed) = reqwest::Url::parse(url) else {
            return refused("keeper.url", "That address isn't valid.");
        };
        let Some(sign_in) = self.sign_in_for(&parsed) else {
            return refused(
                "keeper.host",
                "Provider tokens only go to the providers' own APIs.",
            );
        };
        let Ok(method) = reqwest::Method::from_bytes(method.as_bytes()) else {
            return refused("keeper.method", "That method isn't valid.");
        };
        let Ok(body) = BASE64.decode(body) else {
            return refused("keeper.body", "The request body didn't arrive whole.");
        };
        let token = match self.token(key).await {
            Ok(token) => token,
            Err(response) => return response,
        };
        let signed = match sign_in.headers(&token) {
            Ok(signed) => signed,
            Err(error) => return failed(&error),
        };
        drop(token);
        // The keeper alone signs requests in: one that arrives already carrying
        // a sign-in header is refused rather than merged.
        let signs_in = |name: &str| {
            name.eq_ignore_ascii_case("authorization")
                || signed
                    .iter()
                    .any(|(signed, _)| name.eq_ignore_ascii_case(signed))
        };
        if headers.iter().any(|(name, _)| signs_in(name)) {
            return refused(
                "keeper.authorization",
                "The keeper adds the sign-in itself.",
            );
        }
        let mut request = self.http.request(method, parsed);
        if http2 {
            request = request.version(reqwest::Version::HTTP_2);
        }
        for (name, value) in signed
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .chain(headers)
        {
            request = request.header(name, value);
        }
        let response = match request.body(body).send().await {
            Ok(response) => response,
            Err(error) if error.is_timeout() => {
                return refused("provider.timeout", "The provider didn't answer in time.");
            }
            Err(_) => {
                return refused(
                    "provider.unreachable",
                    "The provider could not be reached. Check your connection and retry; inspect the provider before retrying a failed write.",
                );
            }
        };
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter(|(name, _)| providers::http::RESPONSE_HEADERS.contains(&name.as_str()))
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_owned(), value.to_owned()))
            })
            .collect();
        match response.bytes().await {
            Ok(bytes) if bytes.len() <= MAX_RESPONSE_BYTES => Response::Http {
                status,
                headers,
                body: BASE64.encode(&bytes),
            },
            _ => refused(
                "provider.response_limit",
                "The provider response was too large. Use a narrower query.",
            ),
        }
    }

    async fn provider_cli(
        &self,
        provider: Provider,
        key: &str,
        args: Vec<String>,
        dir: Option<String>,
        stdin: Option<String>,
    ) -> Response {
        let name = provider.display_name();
        // Only the commands the provider's adapter runs.
        let Some(cli) = args
            .first()
            .and_then(|first| providers::keeper_cli(provider, first))
        else {
            return refused(
                "keeper.cli",
                &format!("The keeper only runs Hoista's own {name} commands."),
            );
        };
        let token = match self.token(key).await {
            Ok(token) => token,
            Err(response) => return response,
        };
        let mut command = tool_paths::command(cli.program);
        command.args(&args);
        cli.prepare(&mut command, &token);
        drop(token);
        if let Some(dir) = &dir {
            command.current_dir(dir);
        }
        let input = stdin.map(Secret::new);
        match process::run(&mut command, input.as_ref(), CLI_TIMEOUT).await {
            Ok(status) => Response::Ran {
                success: status.success(),
            },
            Err(ProcessError::Spawn(_)) => refused(
                "provider.cli_missing",
                &format!(
                    "Install {} where Hoista runs before using {name}.",
                    cli.program
                ),
            ),
            Err(_) => refused(
                "provider.cli_failed",
                &format!(
                    "{name} could not complete this operation. Inspect it at {name} before retrying."
                ),
            ),
        }
    }
}

fn failed(error: &EngineError) -> Response {
    match error {
        EngineError::NotFound => refused("keeper.not_found", "Not found."),
        EngineError::Provider { code, message, .. } => refused(code, message),
        other => refused("keeper.vault", &other.to_string()),
    }
}

fn into_done(result: Result<(), EngineError>) -> Response {
    match result {
        Ok(()) => Response::Done,
        Err(error) => failed(&error),
    }
}

#[cfg(unix)]
async fn read_frame(stream: &mut tokio::net::UnixStream) -> std::io::Result<Option<Vec<u8>>> {
    use tokio::io::AsyncReadExt as _;
    let mut length = [0u8; 4];
    match stream.read_exact(&mut length).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = u32::from_be_bytes(length) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(std::io::Error::other("frame too large"));
    }
    let mut bytes = vec![0u8; length];
    stream.read_exact(&mut bytes).await?;
    Ok(Some(bytes))
}

#[cfg(unix)]
async fn write_frame(stream: &mut tokio::net::UnixStream, bytes: &[u8]) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt as _;
    let length = u32::try_from(bytes.len()).map_err(std::io::Error::other)?;
    stream.write_all(&length.to_be_bytes()).await?;
    stream.write_all(bytes).await?;
    stream.flush().await
}

/// The main process's side.
pub struct KeeperClient {
    socket: PathBuf,
}

impl KeeperClient {
    /// Talks to the keeper at `socket`.
    #[must_use]
    pub fn new(socket: PathBuf) -> Self {
        Self { socket }
    }

    async fn call(&self, request: &Request) -> Result<Response, EngineError> {
        #[cfg(unix)]
        {
            let unreachable = || EngineError::Provider {
                evidence: None,
                code: "keeper.unreachable".into(),
                message: "Hoista's credential keeper isn't answering. Restart Hoista.".into(),
            };
            let mut stream = tokio::time::timeout(
                KEEPER_CONNECT_TIMEOUT,
                tokio::net::UnixStream::connect(&self.socket),
            )
            .await
            .map_err(|_| unreachable())?
            .map_err(|_| unreachable())?;
            let bytes = serde_json::to_vec(request).map_err(|_| EngineError::Store)?;
            write_frame(&mut stream, &bytes)
                .await
                .map_err(|_| unreachable())?;
            let reply = tokio::time::timeout(REPLY_TIMEOUT, read_frame(&mut stream))
                .await
                .map_err(|_| unreachable())?
                .map_err(|_| unreachable())?
                .ok_or_else(unreachable)?;
            serde_json::from_slice(&reply).map_err(|_| unreachable())
        }

        #[cfg(not(unix))]
        {
            let _ = request;
            Err(EngineError::Invalid(
                "The credential keeper runs on Linux and macOS only.",
            ))
        }
    }
}

#[async_trait::async_trait]
impl CredentialKeeper for KeeperClient {
    /// Sends a provider request; the keeper signs it in with the token named by
    /// `key`. `http2` sends it over HTTP/2, as gRPC needs.
    async fn http(
        &self,
        key: &str,
        method: &str,
        url: &str,
        headers: Vec<(String, String)>,
        body: &[u8],
        http2: bool,
    ) -> Result<KeptResponse, EngineError> {
        match self
            .call(&Request::Http {
                key: key.into(),
                method: method.into(),
                url: url.into(),
                headers,
                body: BASE64.encode(body),
                http2,
            })
            .await?
        {
            Response::Http {
                status,
                headers,
                body,
            } => Ok(KeptResponse {
                status,
                headers,
                body: BASE64.decode(body).map_err(|_| EngineError::Store)?,
            }),
            other => Err(refusal(other)),
        }
    }

    /// Runs the provider's own command-line tool with the token named by `key`.
    async fn provider_cli(
        &self,
        provider: Provider,
        key: &str,
        args: &[&str],
        dir: Option<&Path>,
        stdin: Option<&Secret>,
    ) -> Result<bool, EngineError> {
        match self
            .call(&Request::ProviderCli {
                provider,
                key: key.into(),
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                dir: dir.map(|dir| dir.to_string_lossy().into_owned()),
                stdin: stdin.map(|input| input.expose().to_owned()),
            })
            .await?
        {
            Response::Ran { success } => Ok(success),
            other => Err(refusal(other)),
        }
    }
}

fn refusal(response: Response) -> EngineError {
    match response {
        Response::Refused { code, .. } if code == "keeper.not_found" => EngineError::NotFound,
        Response::Refused { code, message } => EngineError::Provider {
            evidence: None,
            code,
            message,
        },
        _ => EngineError::Store,
    }
}

/// The main process's vault: provider tokens come back as handles; everything
/// else is read and written through the keeper.
pub struct HandleVault {
    keeper: Arc<KeeperClient>,
}

impl HandleVault {
    /// A vault backed by the keeper.
    #[must_use]
    pub const fn new(keeper: Arc<KeeperClient>) -> Self {
        Self { keeper }
    }
}

#[async_trait::async_trait]
impl TokenVault for HandleVault {
    async fn read(&self, key: &str) -> Result<Secret, EngineError> {
        if provider_token(key) {
            return match self.keeper.call(&Request::Has { key: key.into() }).await? {
                Response::Found { found: true } => Ok(Secret::handle(key)),
                Response::Found { found: false } => Err(EngineError::NotFound),
                other => Err(refusal(other)),
            };
        }
        match self.keeper.call(&Request::Read { key: key.into() }).await? {
            Response::Value { value } => Ok(Secret::new(value)),
            other => Err(refusal(other)),
        }
    }

    async fn write(&self, key: &str, value: Secret) -> Result<(), EngineError> {
        match self
            .keeper
            .call(&Request::Write {
                key: key.into(),
                value: value.expose().to_owned(),
            })
            .await?
        {
            Response::Done => Ok(()),
            other => Err(refusal(other)),
        }
    }

    async fn delete(&self, key: &str) -> Result<(), EngineError> {
        match self
            .keeper
            .call(&Request::Delete { key: key.into() })
            .await?
        {
            Response::Done => Ok(()),
            other => Err(refusal(other)),
        }
    }

    /// A 1Password reference given on connect is held by the keeper, and
    /// checked through it, so its value never comes here.
    async fn resolve(&self, value: &Secret) -> Result<Secret, EngineError> {
        if !value.expose().starts_with(onepassword::SCHEME) {
            return Ok(Secret::new(value.expose().to_owned()));
        }
        match self
            .keeper
            .call(&Request::Hold {
                value: value.expose().to_owned(),
            })
            .await?
        {
            Response::Value { value: key } => Ok(Secret::handle(&key)),
            other => Err(refusal(other)),
        }
    }
}

#[cfg(all(test, unix))]
mod tests;
