//! Connecting a provider account by signing in on the provider's own page (OAuth code
//! with PKCE on a loopback address), and renewing those tokens before expiry.
//!
//! On a server the app relays the callback; that's safe because the code is
//! useless without the PKCE verifier, which never leaves this engine.

use crate::accounts::AccountService;
use crate::engine::{Actor, Engine};
use crate::{
    Provider,
    domain::{Secret, model::*},
    ports::{
        catalog::EngineError,
        providers::{OAuthGrant, OAuthTokens},
    },
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};
use ts_rs::TS;

/// Marks a keychain entry that holds OAuth tokens rather than a pasted token.
const STORED: &str = "hoista-oauth:v1:";
/// How long the provider's page may take before the sign-in gives up.
const WAIT: Duration = Duration::from_secs(300);
/// Renew this many seconds before the access token expires.
const RENEW_BEFORE: i64 = 120;
/// Extra time for the browser's return to arrive once the wait is up.
const CALLBACK_GRACE: Duration = Duration::from_secs(5);
/// The most of the browser's return request that's read.
const MAX_REQUEST_BYTES: usize = 8192;

/// A started sign-in: open `url` in the browser, then finish it with `id`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
pub struct SignInStart {
    /// Pass to finish the sign-in.
    pub id: String,
    /// The provider's authorization page to open in the browser.
    pub url: String,
    /// Only for a relayed sign-in: the port on the person's computer the
    /// browser comes back to, at `/callback`, for the app to forward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub relay_port: Option<u16>,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    access: String,
    refresh: Option<String>,
    /// Unix seconds.
    expires_at: i64,
}

struct Pending {
    provider: Provider,
    verifier: String,
    redirect_uri: String,
    code: oneshot::Receiver<Result<String, EngineError>>,
    started: std::time::Instant,
}

/// Sign-ins waiting for the person, and the renewal lock.
#[derive(Default)]
pub(crate) struct SignIns {
    pending: std::sync::Mutex<HashMap<String, Pending>>,
    /// The listener of the latest sign-in for each provider. Starting again
    /// replaces it, so "open the page again" never runs out of ports.
    listeners: std::sync::Mutex<HashMap<Provider, tokio::task::AbortHandle>>,
    /// Relayed sign-ins waiting for the app to forward the answer: their
    /// state, and where the code goes.
    relays: std::sync::Mutex<HashMap<String, Relay>>,
    renew: tokio::sync::Mutex<()>,
}

struct Relay {
    state: String,
    send: oneshot::Sender<Result<String, EngineError>>,
    started: std::time::Instant,
}

/// 32 random bytes, for a sign-in's `state` and PKCE verifier.
fn random() -> Result<String, EngineError> {
    let mut bytes = [0_u8; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| EngineError::Store)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn person_only(actor: &Actor) -> Result<(), EngineError> {
    if matches!(actor, Actor::Person) {
        Ok(())
    } else {
        Err(EngineError::Conflict(
            "Only the person can connect a provider account, in the Hoista app.",
        ))
    }
}

impl Engine {
    /// The token to send to a provider for an account: a pasted token as saved, or
    /// a signed-in account's access token, renewed first when about to expire.
    ///
    /// # Errors
    /// Reports unavailable keychain entries and renewals the provider refuses.
    pub(crate) async fn credential(&self, account: &Account) -> Result<Secret, EngineError> {
        let holder = account.credential.as_deref().unwrap_or(&account.id);
        let reference = format!("account/{holder}");
        let raw = self.vault.read(&reference).await?;
        let Some(stored) = raw.expose().strip_prefix(STORED) else {
            return Ok(raw);
        };
        let stored: Stored = serde_json::from_str(stored).map_err(|_| EngineError::Store)?;
        if stored.expires_at - RENEW_BEFORE > chrono::Utc::now().timestamp() {
            return Ok(Secret::new(stored.access));
        }
        // One renewal at a time: a rotated refresh token works only once.
        let _renew = self.sign_ins.renew.lock().await;
        let raw = self.vault.read(&reference).await?;
        let stored: Stored = serde_json::from_str(raw.expose().strip_prefix(STORED).unwrap_or(""))
            .map_err(|_| EngineError::Store)?;
        if stored.expires_at - RENEW_BEFORE > chrono::Utc::now().timestamp() {
            return Ok(Secret::new(stored.access));
        }
        let refresh = stored.refresh.ok_or(EngineError::Conflict(
            "This provider's sign-in expired. Connect it again from Accounts.",
        ))?;
        let mut tokens = self
            .adapter(account.provider)?
            .oauth_token(OAuthGrant::Refresh(&refresh))
            .await?;
        // A provider that doesn't rotate keeps the same refresh token.
        tokens.refresh = tokens.refresh.or(Some(Secret::new(refresh)));
        self.accounts().store_tokens(holder, &tokens).await?;
        Ok(tokens.access)
    }
}

impl<'a> AccountService<'a> {
    /// Starts a one-click connection to a provider that has a Hoista OAuth app.
    ///
    /// # Errors
    /// Rejects agents, providers without one-click connection, and busy ports.
    pub async fn start_sign_in(
        self,
        actor: &Actor,
        provider: Provider,
        relay: bool,
    ) -> Result<SignInStart, EngineError> {
        person_only(actor)?;
        let oauth = self
            .engine
            .adapter(provider)?
            .oauth()
            .ok_or(EngineError::Conflict(
                "This provider connects with a token; Hoista can't sign in on its page yet.",
            ))?;
        // A new sign-in for the same provider replaces the one still waiting.
        let previous = self
            .engine
            .sign_ins
            .listeners
            .lock()
            .map_err(|_| EngineError::Store)?
            .remove(&provider);
        if let Some(previous) = previous {
            previous.abort();
            // Let the aborted listener close its port before binding again.
            tokio::task::yield_now().await;
        }
        // Relayed, the app listens on the person's computer: no port here.
        let mut listener = None;
        if relay {
            let port = *oauth.ports.first().ok_or(EngineError::Store)?;
            listener = Some((None, port));
        } else {
            for port in oauth.ports {
                if let Ok(bound) = TcpListener::bind(("127.0.0.1", *port)).await {
                    listener = Some((Some(bound), *port));
                    break;
                }
            }
        }
        let (listener, port) = listener.ok_or(EngineError::Conflict(
            "Another sign-in is already waiting, or its local ports are in use. Finish or close it and try again.",
        ))?;
        let redirect_uri = format!("http://{}:{port}/callback", oauth.redirect_host);
        let state = random()?;
        let verifier = random()?;
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut params = vec![
            ("response_type", "code"),
            ("client_id", oauth.client_id),
            ("redirect_uri", redirect_uri.as_str()),
            ("scope", oauth.scopes),
            ("state", state.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
        ];
        params.extend_from_slice(oauth.extra);
        let url = url::Url::parse_with_params(oauth.authorize_url, &params)
            .map_err(|_| EngineError::Store)?
            .to_string();
        let (send, code) = oneshot::channel();
        let id = uuid::Uuid::new_v4().to_string();
        if let Some(listener) = listener {
            let task = tokio::spawn(wait_for_code(
                listener,
                state,
                (provider.display_name(), oauth.logo),
                self.engine.sign_in_page.clone(),
                send,
            ));
            self.engine
                .sign_ins
                .listeners
                .lock()
                .map_err(|_| EngineError::Store)?
                .insert(provider, task.abort_handle());
        } else {
            let mut relays = self
                .engine
                .sign_ins
                .relays
                .lock()
                .map_err(|_| EngineError::Store)?;
            relays.retain(|_, item| item.started.elapsed() < WAIT);
            relays.insert(
                id.clone(),
                Relay {
                    state,
                    send,
                    started: std::time::Instant::now(),
                },
            );
        }
        let mut pending = self
            .engine
            .sign_ins
            .pending
            .lock()
            .map_err(|_| EngineError::Store)?;
        pending.retain(|_, item| item.started.elapsed() < WAIT * 2);
        pending.insert(
            id.clone(),
            Pending {
                provider,
                verifier,
                redirect_uri,
                code,
                started: std::time::Instant::now(),
            },
        );
        Ok(SignInStart {
            id,
            url,
            relay_port: relay.then_some(port),
        })
    }

    /// For a relayed sign-in: what the browser brought back to the app on
    /// the person's computer. `code` is set when they clicked Allow.
    ///
    /// # Errors
    /// Rejects agents, unknown or finished sign-ins, and a state that doesn't match.
    pub fn relay_callback(
        self,
        actor: &Actor,
        id: &str,
        state: &str,
        code: Option<&str>,
    ) -> Result<(), EngineError> {
        person_only(actor)?;
        let mut relays = self
            .engine
            .sign_ins
            .relays
            .lock()
            .map_err(|_| EngineError::Store)?;
        let fits = relays.get(id).is_some_and(|relay| {
            bool::from(subtle::ConstantTimeEq::ct_eq(
                relay.state.as_bytes(),
                state.as_bytes(),
            ))
        });
        if !fits {
            return Err(EngineError::Conflict(
                "This sign-in link is out of date. Go back to Hoista and connect again.",
            ));
        }
        let relay = relays.remove(id).ok_or(EngineError::NotFound)?;
        let answer = code
            .filter(|code| !code.is_empty())
            .map(str::to_owned)
            .ok_or(EngineError::Conflict(
                "The connection was declined on the provider's page. Nothing was saved.",
            ));
        let _ = relay.send.send(answer);
        Ok(())
    }

    /// Waits for the person to click Allow on the provider's page, then connects
    /// the account they shared (the first, when the provider shares several).
    ///
    /// # Errors
    /// Rejects unknown sign-ins, declined or expired ones, and failed exchanges.
    pub async fn finish_sign_in(self, actor: &Actor, id: &str) -> Result<Account, EngineError> {
        person_only(actor)?;
        let pending = self
            .engine
            .sign_ins
            .pending
            .lock()
            .map_err(|_| EngineError::Store)?
            .remove(id)
            .ok_or(EngineError::NotFound)?;
        let code = tokio::time::timeout(WAIT + CALLBACK_GRACE, pending.code)
            .await
            .map_err(|_| EngineError::Conflict("The sign-in timed out. Try connecting again."))?
            .map_err(|_| {
                EngineError::Conflict("This sign-in was replaced by a newer one, or interrupted. Use the newest page Hoista opened.")
            })??;
        let adapter = self.engine.adapter(pending.provider)?.clone();
        let tokens = adapter
            .oauth_token(OAuthGrant::Code {
                code: &code,
                verifier: &pending.verifier,
                redirect_uri: &pending.redirect_uri,
            })
            .await?;
        let choices = adapter.account_choices(&tokens.access).await?;
        if choices.is_empty() {
            return Err(EngineError::Conflict(
                "Nothing was shared with Hoista. Connect again and choose a workspace on the provider's page.",
            ));
        }
        // Connect every workspace the person shared that this sign-in can read.
        // They share one saved sign-in, held by the first.
        let mut failure = None;
        let mut connected: Vec<Account> = Vec::new();
        for choice in choices {
            match self
                .engine
                .accounts()
                .connect_account(
                    pending.provider,
                    choice.name,
                    choice.external_id,
                    Secret::new(tokens.access.expose().to_owned()),
                )
                .await
            {
                Ok(account) => connected.push(account),
                Err(error) => failure = Some(error),
            }
        }
        let Some(primary) = connected.first().cloned() else {
            return Err(failure.unwrap_or(EngineError::Store));
        };
        self.engine
            .accounts()
            .store_tokens(&primary.id, &tokens)
            .await?;
        // What the person granted on the provider's page, where the provider says,
        // so Hoista knows what this sign-in can do before using it.
        let granted = |account: &mut Account| {
            if !tokens.granted.is_empty() {
                account.scopes.clone_from(&tokens.granted);
            }
        };
        let mut primary = primary;
        granted(&mut primary);
        self.engine.repository.put_account(&primary).await?;
        for mut other in connected.into_iter().skip(1) {
            other.credential = Some(primary.id.clone());
            granted(&mut other);
            self.engine.repository.put_account(&other).await?;
            self.engine
                .vault
                .delete(&format!("account/{}", other.id))
                .await?;
        }
        Ok(primary)
    }

    async fn store_tokens(self, account_id: &str, tokens: &OAuthTokens) -> Result<(), EngineError> {
        let stored = Stored {
            access: tokens.access.expose().to_owned(),
            refresh: tokens
                .refresh
                .as_ref()
                .map(|token| token.expose().to_owned()),
            expires_at: chrono::Utc::now().timestamp() + tokens.expires_in,
        };
        let text = serde_json::to_string(&stored).map_err(|_| EngineError::Store)?;
        self.engine
            .vault
            .write(
                &format!("account/{account_id}"),
                Secret::new(format!("{STORED}{text}")),
            )
            .await
    }
}

/// Serves the one redirect the provider sends back, answering the browser with a
/// short page, and hands over the code when the state matches.
async fn wait_for_code(
    listener: TcpListener,
    state: String,
    brand: (&'static str, &'static str),
    page: Option<std::sync::Arc<dyn crate::ports::sign_in_page::SignInPage>>,
    send: oneshot::Sender<Result<String, EngineError>>,
) {
    let provider_result = |connected: bool, title: &str, text: &str| match &page {
        Some(page) => (
            "text/html",
            page.render(brand.0, brand.1, connected, title, text),
        ),
        None => ("text/plain", format!("{title}\n\n{text}\n")),
    };
    let result = tokio::time::timeout(WAIT, async {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return Err(EngineError::Conflict("The sign-in listener stopped."));
            };
            let mut buffer = vec![0_u8; MAX_REQUEST_BYTES];
            let size = stream.read(&mut buffer).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..size]);
            let target = request
                .lines()
                .next()
                .and_then(|line| line.strip_prefix("GET "))
                .and_then(|line| line.split(' ').next())
                .unwrap_or("");
            let Ok(url) = url::Url::parse(&format!("http://127.0.0.1{target}")) else {
                continue;
            };
            if url.path() != "/callback" {
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await;
                continue;
            }
            let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
            if query.get("state") != Some(&state) {
                respond(
                    &mut stream,
                    provider_result(
                        false,
                        "This sign-in link is out of date",
                        "Go back to Hoista and connect again.",
                    ),
                )
                .await;
                continue;
            }
            if let Some(code) = query.get("code").filter(|code| !code.is_empty()) {
                respond(
                    &mut stream,
                    provider_result(
                        true,
                        &format!("{} is connected", brand.0),
                        "Go back to Hoista. You can close this tab.",
                    ),
                )
                .await;
                return Ok(code.clone());
            }
            respond(
                &mut stream,
                provider_result(
                    false,
                    &format!("{} wasn't connected", brand.0),
                    "Nothing was saved. Go back to Hoista to try again.",
                ),
            )
            .await;
            return Err(EngineError::Conflict(
                "The connection was declined on the provider's page. Nothing was saved.",
            ));
        }
    })
    .await
    .unwrap_or(Err(EngineError::Conflict(
        "Nobody clicked Allow on the provider's page in time. Try connecting again.",
    )));
    let _ = send.send(result);
}

async fn respond(stream: &mut tokio::net::TcpStream, (kind, body): (&str, String)) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {kind}; charset=utf-8\r\nCache-Control: no-store\r\nX-Frame-Options: DENY\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
}
