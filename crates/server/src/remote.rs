//! OAuth and MCP HTTP mapping for agents of Hoista on a server, or on a
//! computer through MCP over HTTP.

use crate::application::accounts::ACCESS_TOKEN_SECONDS;
use crate::{
    application::{
        engine::{Actor, AuthorizationRequest, Engine},
        ports::catalog::EngineError,
    },
    domain::model::{AgentTransport, ApprovalDecision},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

const MAX_BODY_BYTES: usize = crate::MAX_REQUEST_BYTES;
const MAX_PATH_BYTES: usize = 8192;
/// One request passed on to the server. No `Debug`: the body can hold secrets.
pub struct RemoteRequest {
    /// Correlation id.
    pub id: String,
    /// HTTP method.
    pub method: String,
    /// Relative path and query.
    pub path: String,
    /// Restricted transport headers.
    pub headers: BTreeMap<String, String>,
    /// UTF-8 body.
    pub body: String,
}

/// One response to a forwarded request.
pub struct RemoteResponse {
    /// Original correlation id.
    pub id: String,
    /// HTTP status.
    pub status: u16,
    /// Restricted response headers.
    pub headers: BTreeMap<String, String>,
    /// UTF-8 content.
    pub body: String,
}

/// Remote delivery deliberately exposes no desktop or raw provider routes.
pub struct RemoteDispatcher {
    /// Shared engine with transport-independent authorization.
    pub engine: Arc<Engine>,
    /// On a computer (MCP over HTTP), where this computer's own approval
    /// pages are (`http://127.0.0.1:PORT`). The person answers there, never
    /// through the public address. `None` on a server.
    pub local_pages: Option<String>,
}

/// The response header that tells MCP over HTTP where the person reviews
/// a waiting agent on this computer. It never leaves this computer.
pub const REVIEW_HEADER: &str = "x-hoista-review";
impl RemoteDispatcher {
    /// Handles one request without granting local-person authority.
    pub async fn dispatch(&self, request: RemoteRequest) -> RemoteResponse {
        let id = request.id.clone();
        let token_endpoint = request.path.split('?').next() == Some("/oauth/token");
        match self.handle(request).await {
            Ok(response) => response,
            Err(error) => {
                let (status, code, description) = match error {
                    EngineError::AgentsDisabled => (
                        503,
                        "temporarily_unavailable",
                        "Agent access is paused in Hoista.",
                    ),
                    EngineError::Store => (
                        503,
                        "temporarily_unavailable",
                        "Local state is unavailable. Try again later.",
                    ),
                    EngineError::ApprovalRequired | EngineError::NotFound if token_endpoint => (
                        400,
                        "invalid_grant",
                        "The grant is invalid, expired, declined or already used.",
                    ),
                    _ => (
                        400,
                        "invalid_request",
                        "The request is invalid, expired, declined or no longer authorized.",
                    ),
                };
                response(
                    id,
                    status,
                    json!({"error":code,"error_description":description}).to_string(),
                    "application/json",
                )
            }
        }
    }

    async fn handle(&self, request: RemoteRequest) -> Result<RemoteResponse, EngineError> {
        if request.body.len() > MAX_BODY_BYTES || request.path.len() > MAX_PATH_BYTES {
            return Err(EngineError::Invalid("Request too large."));
        }
        let public = self
            .engine
            .public_url()
            .ok_or(EngineError::NotFound)?
            .to_owned();
        let (path, query) = request.path.split_once('?').unwrap_or((&request.path, ""));
        match (request.method.as_str(), path) {
            ("POST", "/oauth/register") => {
                #[derive(Deserialize)]
                struct Registration {
                    client_name: String,
                    redirect_uris: Vec<String>,
                    token_endpoint_auth_method: Option<String>,
                }
                let input: Registration = serde_json::from_str(&request.body)
                    .map_err(|_| EngineError::Invalid("Invalid client registration."))?;
                if input
                    .token_endpoint_auth_method
                    .as_deref()
                    .is_some_and(|method| method != "none")
                {
                    return Err(EngineError::Invalid(
                        "Only public PKCE clients are supported.",
                    ));
                }
                let client = self
                    .engine
                    .accounts()
                    .oauth_register(input.client_name, input.redirect_uris)
                    .await?;
                Ok(response(
                    request.id,
                    201,
                    json!({
                        "client_id": client.id,
                        "client_name": client.name,
                        "redirect_uris": client.redirect_uris,
                        "token_endpoint_auth_method": "none",
                        "grant_types": ["authorization_code"],
                        "response_types": ["code"],
                    })
                    .to_string(),
                    "application/json",
                ))
            }
            ("GET", "/oauth/authorize") => {
                let params = params(query)?;
                if let Some(code) = params.get("code") {
                    let (pending, decision) = self.engine.accounts().oauth_pending(code).await?;
                    if decision == ApprovalDecision::Pending {
                        if self.engine.is_server() {
                            // On a server the same browser signs in and answers.
                            return Ok(response(
                                request.id,
                                200,
                                cloud_waiting_page(&public, &pending.id, code),
                                "text/html; charset=utf-8",
                            ));
                        }
                        // On a computer the person answers on the computer
                        // itself; this page waits and then carries on.
                        let review = self
                            .local_pages
                            .as_deref()
                            .map(|pages| format!("{pages}/connect/oauth-{}", pending.id));
                        let mut result = response(
                            request.id,
                            200,
                            link_waiting_page(&pending.id),
                            "text/html; charset=utf-8",
                        );
                        if let Some(review) = review {
                            result.headers.insert(REVIEW_HEADER.into(), review);
                        }
                        return Ok(result);
                    }
                    let mut redirect =
                        url::Url::parse(&pending.redirect_uri).map_err(|_| EngineError::Store)?;
                    if decision == ApprovalDecision::Allowed {
                        redirect
                            .query_pairs_mut()
                            .append_pair("code", code)
                            .append_pair("state", &pending.state);
                    } else {
                        redirect
                            .query_pairs_mut()
                            .append_pair("error", "access_denied")
                            .append_pair("state", &pending.state);
                    }
                    let mut result = response(request.id, 302, String::new(), "text/plain");
                    result.headers.insert("location".into(), redirect.into());
                    Ok(result)
                } else {
                    if required(&params, "response_type")? != "code"
                        || required(&params, "code_challenge_method")? != "S256"
                    {
                        return Err(EngineError::Invalid(
                            "Use authorization code with S256 PKCE.",
                        ));
                    }
                    let (_, code) = self
                        .engine
                        .accounts()
                        .oauth_authorize(AuthorizationRequest {
                            client_id: required(&params, "client_id")?.into(),
                            redirect_uri: required(&params, "redirect_uri")?.into(),
                            state: required(&params, "state")?.into(),
                            challenge: required(&params, "code_challenge")?.into(),
                            resource: required(&params, "resource")?.into(),
                            scopes: params
                                .get("scope")
                                .map(String::as_str)
                                .unwrap_or("apps:read")
                                .split_whitespace()
                                .map(str::to_owned)
                                .collect(),
                        })
                        .await?;
                    let mut redirect = url::Url::parse(&format!("{public}/oauth/authorize"))
                        .map_err(|_| EngineError::Store)?;
                    redirect
                        .query_pairs_mut()
                        .append_pair("code", code.expose());
                    let mut result = response(request.id, 303, String::new(), "text/plain");
                    result.headers.insert("location".into(), redirect.into());
                    Ok(result)
                }
            }
            ("POST", "/oauth/token") => {
                let params = params(&request.body)?;
                if required(&params, "grant_type")? != "authorization_code" {
                    return Err(EngineError::Invalid("Unsupported grant."));
                }
                let token = self
                    .engine
                    .accounts()
                    .oauth_token(
                        required(&params, "code")?,
                        required(&params, "client_id")?,
                        required(&params, "redirect_uri")?,
                        required(&params, "code_verifier")?,
                        required(&params, "resource")?,
                    )
                    .await?;
                Ok(response(
                    request.id,
                    200,
                    json!({"access_token": token.expose(), "token_type": "Bearer", "expires_in": ACCESS_TOKEN_SECONDS})
                        .to_string(),
                    "application/json",
                ))
            }
            ("POST", "/mcp") => {
                let unauthorized = || {
                    let mut result = response(
                        request.id.clone(),
                        401,
                        json!({"error":"invalid_token"}).to_string(),
                        "application/json",
                    );
                    result.headers.insert(
                        "www-authenticate".into(),
                        format!("Bearer resource_metadata=\"{public}/.well-known/oauth-protected-resource\""),
                    );
                    result
                };
                let Some(credential) = request
                    .headers
                    .get("authorization")
                    .and_then(|value| value.strip_prefix("Bearer "))
                else {
                    return Ok(unauthorized());
                };
                let actor = match self.engine.agents().authenticate_agent(credential).await {
                    Ok(actor) => actor,
                    Err(error @ EngineError::AgentsDisabled) => return Err(error),
                    Err(_) => return Ok(unauthorized()),
                };
                let Actor::Agent { id } = &actor else {
                    return Ok(unauthorized());
                };
                let agent = self
                    .engine
                    .repository
                    .agent(id)
                    .await?
                    .ok_or(EngineError::NotFound)?;
                if agent.transport != AgentTransport::Link {
                    return Ok(unauthorized());
                }
                if let Some(origin) = request.headers.get("origin") {
                    let grant = self
                        .engine
                        .repository
                        .oauth_grant(id)
                        .await?
                        .ok_or(EngineError::ApprovalRequired)?;
                    let client = self
                        .engine
                        .repository
                        .oauth_client(&grant.client_id)
                        .await?
                        .ok_or(EngineError::ApprovalRequired)?;
                    if *origin != public
                        && !client.redirect_uris.iter().any(|uri| {
                            url::Url::parse(uri)
                                .is_ok_and(|uri| uri.origin().ascii_serialization() == *origin)
                        })
                    {
                        return Ok(response(
                            request.id,
                            403,
                            "Invalid origin".into(),
                            "text/plain",
                        ));
                    }
                }
                let message: Value = serde_json::from_str(&request.body)
                    .map_err(|_| EngineError::Invalid("Invalid JSON-RPC body."))?;
                let reply = super::mcp::dispatch(&self.engine, &actor, message).await;
                Ok(response(
                    request.id,
                    if reply.is_some() { 200 } else { 202 },
                    reply.map(|reply| reply.to_string()).unwrap_or_default(),
                    "application/json",
                ))
            }
            (_, "/mcp") => {
                let mut result = response(
                    request.id,
                    405,
                    "Use POST for this sessionless MCP endpoint.".into(),
                    "text/plain",
                );
                result.headers.insert("allow".into(), "POST".into());
                Ok(result)
            }
            _ => Ok(response(request.id, 404, "Not found".into(), "text/plain")),
        }
    }
}

fn response(id: String, status: u16, body: String, content_type: &str) -> RemoteResponse {
    RemoteResponse {
        id,
        status,
        body,
        headers: BTreeMap::from([
            ("content-type".into(), content_type.into()),
            ("cache-control".into(), "no-store".into()),
            ("referrer-policy".into(), "no-referrer".into()),
            (
                "content-security-policy".into(),
                "default-src 'none'; frame-ancestors 'none'".into(),
            ),
        ]),
    }
}

fn params(input: &str) -> Result<BTreeMap<String, String>, EngineError> {
    let mut result = BTreeMap::new();
    for (key, value) in url::form_urlencoded::parse(input.as_bytes()) {
        if result
            .insert(key.into_owned(), value.into_owned())
            .is_some()
        {
            return Err(EngineError::Invalid(
                "Duplicate OAuth parameters are not allowed.",
            ));
        }
    }
    Ok(result)
}

fn required<'a>(params: &'a BTreeMap<String, String>, name: &str) -> Result<&'a str, EngineError> {
    params
        .get(name)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(EngineError::Invalid("Missing OAuth parameter."))
}

/// On a server: the browser that started the sign-in signs in to Hoista and
/// answers right away, then comes back here and on to the agent.
fn cloud_waiting_page(public: &str, pending: &str, code: &str) -> String {
    let back = format!("/oauth/authorize?code={code}");
    let review = format!(
        "{public}/connect/oauth-{pending}?back={}",
        url::form_urlencoded::byte_serialize(back.as_bytes()).collect::<String>()
    );
    crate::views::render(&crate::views::AgentWaitingPage {
        title: "Connect to Hoista",
        code: &crate::application::engine::pairing_code(pending),
        review: Some(&review),
    })
}

/// On a computer (MCP over HTTP): the person allows the agent on that
/// computer, where Hoista asks for Touch ID or the password. This page, which
/// may be open anywhere, only shows the code to match and reloads until the
/// answer is in, then goes on to the agent.
fn link_waiting_page(pending: &str) -> String {
    crate::views::render(&crate::views::AgentWaitingPage {
        title: "Connect to Hoista",
        code: &crate::application::engine::pairing_code(pending),
        review: None,
    })
}
