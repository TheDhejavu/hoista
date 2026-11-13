//! The MCP stdio bridge between a coding tool and the daemon, as size-limited
//! newline-delimited JSON-RPC. Only JSON-RPC messages may reach stdout.

use super::client::{ClientError, LocalClient};
use super::mcp::rpc_error;
use crate::domain::Secret;
use crate::rpc_code;
use axum::http::Method;
use hoista_system::paths::StatePaths;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

/// The largest MCP message read from the coding tool.
const MAX_MESSAGE_BYTES: usize = crate::MAX_REQUEST_BYTES;
const RECONNECT_WAIT: std::time::Duration = std::time::Duration::from_secs(1);
/// Bytes of a hash shown as a short id.
const SHORT_DIGEST_BYTES: usize = 12;

/// Runs an MCP stdio session as a thin client of the existing daemon.
///
/// A coding tool is authorized when it connects, the way an OAuth sign-in asks
/// before anything else: the session starts and lists tools at once (so the
/// client never times out), and in the background the bridge asks Hoista to
/// connect and opens Hoista's page in the browser, where the person checks
/// the code and picks the access. A tool call made before then waits for the
/// answer. The credential is kept in the system keychain under this
/// connection's label, so later sessions reuse it without asking. The daemon
/// never hands out a credential because a name matches.
///
/// # Errors
/// Reports unavailable daemon, enrollment failure or broken standard IO.
pub async fn run(
    paths: &StatePaths,
    name: Option<&str>,
    vault: std::sync::Arc<dyn crate::application::ports::vault::TokenVault>,
) -> Result<(), ClientError> {
    let client = LocalClient::new(paths)?;
    let mut credential: Option<Secret> = None;
    let mut input = Incoming::new();
    let mut output = tokio::io::stdout();
    let mut initialized = false;
    let mut label_seen: Option<String> = None;
    // Whether the client can ask the person a question (MCP elicitation).
    let mut can_ask = false;
    let mut client_name = "your agent".to_owned();
    // Coding tools start their MCP servers in the folder they work in.
    let folder = std::env::current_dir()
        .ok()
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.display().to_string());
    // Asking to connect, in the background from the moment the session starts.
    let mut asking: Option<tokio::sync::watch::Receiver<Asking>> = None;
    loop {
        let mut request = match input.next(&mut output).await? {
            None => return Ok(()),
            Some(request) => request,
        };
        // Say which folder the coding tool works in, so Hoista knows its
        // project. Whatever the tool put there itself is replaced.
        if request["method"] == "tools/call"
            && let Some(params) = request["params"].as_object_mut()
        {
            let meta = params.entry("_meta").or_insert_with(|| json!({}));
            if let (Some(meta), Some(folder)) = (meta.as_object_mut(), &folder) {
                meta.insert(super::mcp::FOLDER_META.into(), json!(folder));
            }
        }
        if request["method"] == "initialize" {
            can_ask = request["params"]["capabilities"]["elicitation"].is_object();
            if let Some(name) = request["params"]["clientInfo"]["name"].as_str() {
                client_name = name.to_owned();
            }
        }
        let notification = request.get("id").is_none()
            && request["jsonrpc"] == "2.0"
            && request["method"].is_string();
        if notification {
            continue;
        }
        if !initialized && request["method"] != "initialize" {
            write(
                &mut output,
                &rpc_error(
                    request.get("id").cloned().unwrap_or(Value::Null),
                    rpc_code::NOT_INITIALIZED,
                    "Initialize this MCP session first.",
                ),
            )
            .await?;
            continue;
        }
        if credential.is_none() {
            let label = label_seen
                .get_or_insert_with(|| {
                    name.or_else(|| request["params"]["clientInfo"]["name"].as_str())
                        .unwrap_or("Coding agent")
                        .to_owned()
                })
                .clone();
            if request["method"] != "tools/call" {
                // Starting the session, ping and the tool list answer at once.
                let response: Value = client
                    .authenticated(Method::POST, "/mcp", Some(&request), None)
                    .await?;
                if request["method"] == "initialize" && response.get("result").is_some() {
                    initialized = true;
                    // Ask now, as the session starts, not at the first tool call.
                    asking.get_or_insert_with(|| {
                        ask(paths.clone(), label.clone(), can_ask, vault.clone())
                    });
                }
                write(&mut output, &response).await?;
                continue;
            }
            let receiver = asking
                .get_or_insert_with(|| ask(paths.clone(), label.clone(), can_ask, vault.clone()));
            // A tool call before the person answers waits for the answer.
            let _ = tokio::time::timeout(
                WAIT_FOR_ANSWER,
                receiver.wait_for(|state| matches!(state, Asking::Ready(_) | Asking::Stopped(_))),
            )
            .await;
            let state = receiver.borrow().clone();
            match state {
                Asking::Ready(secret) => credential = Some(Secret::new(secret.expose().to_owned())),
                Asking::Stopped(message) => {
                    // Ended (not allowed, expired): the next tool call asks again.
                    asking = None;
                    tool_error(&mut output, &request, &message).await?;
                    continue;
                }
                Asking::Waiting(message) => {
                    tool_error(&mut output, &request, &message).await?;
                    continue;
                }
                Asking::Starting => {
                    tool_error(
                        &mut output,
                        &request,
                        "Hoista is still opening its page to ask the person. Try again in a moment.",
                    )
                    .await?;
                    continue;
                }
            }
        }
        let response: Value = match client
            .authenticated(
                Method::POST,
                "/mcp",
                Some(&request),
                Some(credential.as_ref().ok_or(ClientError::Transport)?.expose()),
            )
            .await
        {
            Ok(response) => response,
            // Paused, or the keychain is locked: say so and keep the session.
            Err(ClientError::Rejected(message)) => {
                let id = request.get("id").cloned().unwrap_or(Value::Null);
                rpc_error(id, rpc_code::UNAVAILABLE, &message)
            }
            // No usable answer. That's a revoked connection only if the
            // credential no longer works; a slow call or a restarting Hoista
            // keeps it, so the person isn't asked to allow the agent again.
            Err(ClientError::Transport) => {
                let id = request.get("id").cloned().unwrap_or(Value::Null);
                let message = match still_allowed(&client, credential.as_ref()).await {
                    Allowed::Yes => {
                        "Hoista didn't answer this call in time. It may still be running: check the project before trying a change again."
                    }
                    Allowed::Unreachable => {
                        "Hoista isn't answering right now; it may be restarting. Try again in a moment."
                    }
                    Allowed::No => {
                        credential = None;
                        asking = None;
                        "This connection to Hoista ended. The next tool call asks the person to allow it again."
                    }
                };
                rpc_error(id, rpc_code::UNAVAILABLE, message)
            }
        };
        if request["method"] == "initialize" && response.get("result").is_some() {
            initialized = true;
        }
        let response = match (pending_approval(&response), credential.as_ref()) {
            (Some(approval), Some(secret)) if request["method"] == "tools/call" => {
                answer_here(
                    Hoista {
                        client: &client,
                        secret,
                    },
                    AgentClient {
                        name: &client_name,
                        can_ask,
                    },
                    &mut input,
                    &mut output,
                    &request,
                    response.clone(),
                    &approval,
                )
                .await?
            }
            _ => response,
        };
        write(&mut output, &response).await?;
    }
}

/// The approval a tool call is waiting on, if it returned pending.
/// Whether a connection still works after a call got no usable answer.
enum Allowed {
    Yes,
    No,
    /// Hoista itself isn't answering, so nothing can be said about it.
    Unreachable,
}

async fn still_allowed(client: &LocalClient, credential: Option<&Secret>) -> Allowed {
    let Some(credential) = credential else {
        return Allowed::No;
    };
    if client.health().await.is_err() {
        return Allowed::Unreachable;
    }
    match client
        .authenticated::<Value>(
            Method::POST,
            "/mcp",
            Some(&json!({"jsonrpc":"2.0","id":0,"method":"ping"})),
            Some(credential.expose()),
        )
        .await
    {
        // Answered, or refused with a reason (paused): still this agent's.
        Ok(_) | Err(ClientError::Rejected(_)) => Allowed::Yes,
        Err(ClientError::Transport) => Allowed::No,
    }
}

fn pending_approval(response: &Value) -> Option<String> {
    let result = &response["result"]["structuredContent"];
    (result["status"] == "pending")
        .then(|| result["approval_id"].as_str().map(str::to_owned))
        .flatten()
}

/// Asks the person in their own client (MCP elicitation) whether to allow
/// what the agent asked for, passes the answer to Hoista, and, when they
/// allowed it, runs the call again with the approval. Returns what the agent
/// sees as the call's result.
async fn answer_here(
    hoista: Hoista<'_>,
    agent: AgentClient<'_>,
    input: &mut Incoming,
    output: &mut tokio::io::Stdout,
    request: &Value,
    pending: Value,
    approval_id: &str,
) -> Result<Value, ClientError> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let said = |text: &str| {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{"type": "text", "text": text}],
                "isError": false,
                "structuredContent": {"status": "not_allowed", "message": text},
            },
        })
    };
    if !agent.can_ask {
        return Ok(said(&format!(
            "{} can't show Hoista's question, so nothing was changed. Tell the person: they can let this agent do these without asking in Hoista → Agents, under \"Changes that need your OK\".",
            agent.name
        )));
    }
    let read = hoista
        .call(&json!({
            "jsonrpc": "2.0",
            "id": "hoista-approval",
            "method": "tools/call",
            "params": {"name": "get_approval", "arguments": {"id": approval_id}},
        }))
        .await?;
    let approval = &read["result"]["structuredContent"];
    let typed = approval["deletion"]["confirmation_name"].as_str();
    // A deletion says what goes; anything else uses the reason Hoista gave.
    let listed: Vec<String> = approval["deletion"]["resources"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(format!(
                        "{} ({})",
                        item["name"].as_str()?,
                        item["kind"].as_str().unwrap_or("resource")
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let reason = match typed {
        Some(name) if !listed.is_empty() => {
            format!(
                "Delete the project {name} and everything in it: {}.",
                listed.join(", ")
            )
        }
        Some(name) => format!("Delete the project {name}."),
        None => approval["reason"]
            .as_str()
            .unwrap_or("Make a change you have to allow")
            .to_owned(),
    };
    let message = match typed {
        Some(name) => format!(
            "Hoista: {reason}\n\nThis can't be undone. Type {name} to allow it, then confirm it's you with Touch ID or your password."
        ),
        None => format!(
            "Hoista: {reason}\n\nAllow this? You'll confirm it's you with Touch ID or your password."
        ),
    };
    let schema = match typed {
        Some(name) => json!({
            "type": "object",
            "properties": {"name": {"type": "string", "title": format!("Type {name} to confirm")}},
            "required": ["name"],
        }),
        None => json!({
            "type": "object",
            "properties": {"allow": {"type": "boolean", "title": "Allow it", "default": true}},
        }),
    };
    let asked = input.ask(output, &message, schema).await?;
    let action = asked["result"]["action"].as_str().unwrap_or("cancel");
    if action == "cancel" {
        return Ok(said(
            "The person closed Hoista's question without answering, so nothing was changed. Ask them before trying again.",
        ));
    }
    let allow = action == "accept" && asked["result"]["content"]["allow"] != false;
    let answered = hoista
        .call(&json!({"jsonrpc":"2.0","id":"hoista-answer","method":"hoista/answer_approval","params":{
            "approval_id": approval_id,
            "allow": allow,
            "name": asked["result"]["content"]["name"],
        }}))
        .await?;
    if let Some(message) = answered["error"]["message"].as_str() {
        return Ok(said(&if message.contains("nothing") {
            message.to_owned()
        } else {
            format!("{message} Nothing was changed.")
        }));
    }
    if !allow {
        return Ok(said(
            "The person said no, so nothing was changed. Don't try again unless they ask.",
        ));
    }
    // Allowed: run the same call again with the approval.
    let mut again = request.clone();
    again["params"]["arguments"]["approval_id"] = json!(approval_id);
    Ok(hoista.call(&again).await.unwrap_or(pending))
}

/// Hoista's MCP endpoint, called with this agent's credential.
#[derive(Clone, Copy)]
struct Hoista<'a> {
    client: &'a LocalClient,
    secret: &'a Secret,
}

impl Hoista<'_> {
    async fn call(&self, body: &Value) -> Result<Value, ClientError> {
        self.client
            .authenticated(Method::POST, "/mcp", Some(body), Some(self.secret.expose()))
            .await
    }
}

/// The agent's own client, as it described itself when it connected.
struct AgentClient<'a> {
    name: &'a str,
    /// Whether it can show the person a question (MCP elicitation).
    can_ask: bool,
}

/// Messages from the client, one JSON-RPC object per line. While the bridge
/// waits for the person's answer to a question, other requests the client
/// sends are kept and handled afterwards, in order.
struct Incoming {
    input: tokio::io::BufReader<tokio::io::Stdin>,
    kept: std::collections::VecDeque<Value>,
    asked: u64,
}

/// How long the person has to answer Hoista's question in their client.
const ANSWER_WITHIN: std::time::Duration = std::time::Duration::from_secs(10 * 60);

impl Incoming {
    fn new() -> Self {
        Self {
            input: tokio::io::BufReader::new(tokio::io::stdin()),
            kept: std::collections::VecDeque::new(),
            asked: 0,
        }
    }

    /// The next request, or None when the client closed the session.
    /// Notifications and replies nobody waits for are skipped.
    async fn next(&mut self, output: &mut tokio::io::Stdout) -> Result<Option<Value>, ClientError> {
        if let Some(kept) = self.kept.pop_front() {
            return Ok(Some(kept));
        }
        loop {
            let Some(message) = self.read(output).await? else {
                return Ok(None);
            };
            if message["method"].is_string() && message.get("id").is_some() {
                return Ok(Some(message));
            }
        }
    }

    async fn read(&mut self, output: &mut tokio::io::Stdout) -> Result<Option<Value>, ClientError> {
        loop {
            let mut line = Vec::new();
            let read = (&mut self.input)
                .take(MAX_MESSAGE_BYTES as u64 + 1)
                .read_until(b'\n', &mut line)
                .await
                .map_err(|_| ClientError::Transport)?;
            if read == 0 {
                return Ok(None);
            }
            if read > MAX_MESSAGE_BYTES {
                return Err(ClientError::Rejected("MCP message exceeds 128 KiB.".into()));
            }
            match serde_json::from_slice::<Value>(&line) {
                Ok(value) => return Ok(Some(value)),
                Err(_) => {
                    write(
                        output,
                        &rpc_error(Value::Null, rpc_code::PARSE_ERROR, "Invalid JSON."),
                    )
                    .await?;
                }
            }
        }
    }

    /// Asks the person a question through the client and waits for the reply.
    async fn ask(
        &mut self,
        output: &mut tokio::io::Stdout,
        message: &str,
        schema: Value,
    ) -> Result<Value, ClientError> {
        self.asked += 1;
        let id = format!("hoista-question-{}", self.asked);
        write(
            output,
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "elicitation/create",
                "params": {"message": message, "requestedSchema": schema},
            }),
        )
        .await?;
        let waited = tokio::time::timeout(ANSWER_WITHIN, async {
            loop {
                let Some(message) = self.read(output).await? else {
                    return Err(ClientError::Transport);
                };
                if message["id"] == id.as_str() && message.get("method").is_none() {
                    return Ok(message);
                }
                if message["method"].is_string() && message.get("id").is_some() {
                    self.kept.push_back(message);
                }
            }
        })
        .await;
        match waited {
            Ok(answer) => answer,
            Err(_) => Ok(json!({"result":{"action":"cancel"}})),
        }
    }
}

/// How long a tool call waits for the person to authorize the connection.
const WAIT_FOR_ANSWER: std::time::Duration = std::time::Duration::from_secs(60);
/// How long the background request keeps checking before it gives up.
const ASK_FOR: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Where asking to connect has got to.
#[derive(Clone)]
enum Asking {
    Starting,
    /// Not authorized yet: what to tell the client.
    Waiting(String),
    Ready(std::sync::Arc<Secret>),
    /// Not allowed, or expired.
    Stopped(String),
}

/// Asks Hoista to connect in the background and keeps checking for the
/// answer. A saved credential that still works is used straight away.
fn ask(
    paths: StatePaths,
    label: String,
    can_ask: bool,
    vault: std::sync::Arc<dyn crate::application::ports::vault::TokenVault>,
) -> tokio::sync::watch::Receiver<Asking> {
    let (sender, receiver) = tokio::sync::watch::channel(Asking::Starting);
    tokio::spawn(async move {
        let Ok(client) = LocalClient::new(&paths) else {
            sender.send_replace(Asking::Stopped(
                "Hoista isn't running on this computer.".into(),
            ));
            return;
        };
        let page = super::pages::addresses(&paths).map(|found| found.local);
        let mut asked: Option<(String, String, String)> = None;
        let started = std::time::Instant::now();
        loop {
            match connection(
                paths.root(),
                &client,
                vault.as_ref(),
                &label,
                &mut asked,
                page.as_deref(),
                can_ask,
            )
            .await
            {
                Ok(Connection::Ready(secret)) => {
                    sender.send_replace(Asking::Ready(std::sync::Arc::new(secret)));
                    return;
                }
                Ok(Connection::Denied(message)) => {
                    sender.send_replace(Asking::Stopped(message));
                    return;
                }
                Ok(Connection::Waiting(message)) => {
                    sender.send_replace(Asking::Waiting(message));
                }
                // Paused, or another refusal: the tool call says so at once
                // and the next one asks again.
                Err(ClientError::Rejected(message)) => {
                    sender.send_replace(Asking::Stopped(message));
                    return;
                }
                Err(_) => {
                    sender.send_replace(Asking::Waiting(
                        "Hoista isn't answering right now. Make sure it's running.".into(),
                    ));
                }
            }
            if sender.is_closed() {
                return;
            }
            if started.elapsed() >= ASK_FOR {
                sender.send_replace(Asking::Stopped(
                    "The request to connect expired. The next tool call asks again.".into(),
                ));
                return;
            }
            tokio::time::sleep(RECONNECT_WAIT).await;
        }
    });
    receiver
}

async fn tool_error(
    output: &mut tokio::io::Stdout,
    request: &Value,
    message: &str,
) -> Result<(), ClientError> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    write(
        output,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {"content": [{"type": "text", "text": message}], "isError": true},
        }),
    )
    .await
}

/// Opens Hoista's page in the default browser; best effort.
pub fn open_in_browser(url: &str) {
    if std::env::var_os("HOISTA_NO_BROWSER").is_some() {
        return;
    }
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(program)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

async fn write(output: &mut tokio::io::Stdout, response: &Value) -> Result<(), ClientError> {
    let mut bytes = serde_json::to_vec(response).map_err(|_| ClientError::Transport)?;
    bytes.push(b'\n');
    output
        .write_all(&bytes)
        .await
        .map_err(|_| ClientError::Transport)?;
    output.flush().await.map_err(|_| ClientError::Transport)
}

enum Connection {
    Ready(Secret),
    /// Not allowed yet: what to tell the client.
    Waiting(String),
    /// The person said no.
    Denied(String),
}

/// Keychain entries for this connection: one per Hoista installation (its
/// state directory) and label, by a digest so neither is spelled out in a key.
fn keys(installation: &std::path::Path, label: &str) -> (String, String) {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(installation.as_os_str().as_encoded_bytes());
    hasher.update([0]);
    hasher.update(label.as_bytes());
    let digest = hasher.finalize();
    let short: String = digest
        .iter()
        .take(SHORT_DIGEST_BYTES)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    (
        format!("connection/{short}"),
        format!("connection/{short}/pending"),
    )
}

/// A saved credential that still works, a newly allowed one, or what the
/// person still has to do.
async fn connection(
    installation: &std::path::Path,
    client: &LocalClient,
    vault: &dyn crate::application::ports::vault::TokenVault,
    label: &str,
    asked: &mut Option<(String, String, String)>,
    page: Option<&str>,
    can_ask: bool,
) -> Result<Connection, ClientError> {
    let (saved_key, pending_key) = keys(installation, label);
    if let Ok(saved) = vault.read(&saved_key).await {
        match client
            .authenticated::<Value>(
                Method::POST,
                "/mcp",
                Some(&json!({"jsonrpc":"2.0","id":0,"method":"ping"})),
                Some(saved.expose()),
            )
            .await
        {
            Ok(_) => return Ok(Connection::Ready(saved)),
            // Paused or another refusal the person should read: say so at
            // once and keep the credential; the next tool call checks again.
            Err(ClientError::Rejected(message)) => return Ok(Connection::Denied(message)),
            // Revoked or ended: forget it and ask again; the person decides.
            Err(ClientError::Transport) => {
                client.health().await?;
                let _ = vault.delete(&saved_key).await;
            }
        }
    }
    let mut pending: Option<(String, String, String)> = match asked.clone() {
        Some(found) => Some(found),
        None => vault
            .read(&pending_key)
            .await
            .ok()
            .and_then(|value| serde_json::from_str::<Value>(value.expose()).ok())
            .and_then(|value| {
                Some((
                    value["id"].as_str()?.to_owned(),
                    value["channel"].as_str()?.to_owned(),
                    value["code"].as_str()?.to_owned(),
                ))
            }),
    };
    loop {
        let (id, channel, code) = match pending.clone() {
            Some(found) => found,
            None => {
                let created: Value = client
                    .request(
                        Method::POST,
                        "/agents/enrollments",
                        Some(&json!({"name": label, "can_ask": can_ask})),
                    )
                    .await?;
                let id = created["request"]["id"]
                    .as_str()
                    .ok_or(ClientError::Transport)?
                    .to_owned();
                let code = created["request"]["code"]
                    .as_str()
                    .ok_or(ClientError::Transport)?
                    .to_owned();
                let channel = created["channel"]
                    .as_str()
                    .ok_or(ClientError::Transport)?
                    .to_owned();
                // Without a usable keychain the request still works for this
                // session; the next session just asks again.
                if vault
                    .write(
                        &pending_key,
                        Secret::new(
                            json!({"id": id, "channel": channel, "code": code}).to_string(),
                        ),
                    )
                    .await
                    .is_err()
                {
                    eprintln!(
                        "Hoista: the keychain isn't available, so this request lasts only for this session."
                    );
                }
                match page {
                    Some(base) => {
                        let url = format!("{base}/connect/{id}");
                        eprintln!(
                            "Hoista: \"{label}\" is asking to connect. Authorize it at {url} (code {code})."
                        );
                        open_in_browser(&url);
                    }
                    None => eprintln!(
                        "Hoista: \"{label}\" is asking to connect. Allow it in Hoista → Agents, or run `hoist agents allow`. Code: {code}"
                    ),
                }
                *asked = Some((id.clone(), channel.clone(), code.clone()));
                (id, channel, code)
            }
        };
        let answer: Result<Value, ClientError> = client
            .request(
                Method::POST,
                &format!("/agents/enrollments/{id}/poll"),
                Some(&json!({"channel": channel})),
            )
            .await;
        match answer {
            Ok(answer) => match answer["status"].as_str() {
                Some("allowed") => {
                    let Some(value) = answer["credential"].as_str() else {
                        return Err(ClientError::Transport);
                    };
                    let secret = Secret::new(value.to_owned());
                    if vault
                        .write(&saved_key, Secret::new(value.to_owned()))
                        .await
                        .is_err()
                    {
                        eprintln!(
                            "Hoista: the keychain isn't available, so the next session will ask again."
                        );
                    }
                    let _ = vault.delete(&pending_key).await;
                    *asked = None;
                    eprintln!("Hoista: \"{label}\" is connected.");
                    return Ok(Connection::Ready(secret));
                }
                Some("pending") => {}
                Some("denied") => {
                    let _ = vault.delete(&pending_key).await;
                    *asked = None;
                    return Ok(Connection::Denied(
                        "The person didn't authorize this connection. Calling a tool again asks again.".into(),
                    ));
                }
                // Expired or already collected elsewhere: ask again.
                _ => {
                    let _ = vault.delete(&pending_key).await;
                    *asked = None;
                    pending = None;
                    continue;
                }
            },
            // The daemon restarted or the request ended: ask again.
            Err(ClientError::Rejected(_)) => {
                let _ = vault.delete(&pending_key).await;
                *asked = None;
                pending = None;
                continue;
            }
            Err(error) => return Err(error),
        }
        let place = page.map_or_else(
            || "in Hoista → Agents".to_owned(),
            |base| format!("at {base}/connect/{id}"),
        );
        return Ok(Connection::Waiting(format!(
            "Waiting for the person to authorize \"{label}\" {place} (code {code})."
        )));
    }
}
