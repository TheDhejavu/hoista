//! Live Worker logs through Cloudflare's tail WebSocket (`trace-v1`). Request
//! metadata (headers, URLs, client IPs, bodies) is never passed on.

use super::{Cloudflare, invalid};
use crate::{
    application::ports::{catalog::EngineError, providers::LogSession},
    domain::{
        Secret,
        leaks::LeakScanner,
        logs::{LogEntry, LogLevel, MAX_MESSAGE_BYTES},
    },
};
use futures_util::{SinkExt, StreamExt};
use reqwest::Method;
use serde_json::{Value, json};
use std::{collections::VecDeque, time::Duration};
use tokio::{
    net::TcpStream,
    time::{Instant, timeout},
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async_with_config,
    tungstenite::{
        client::IntoClientRequest,
        protocol::{Message, WebSocketConfig},
    },
};

const MAX_QUERY_BYTES: usize = 256;
const MAX_ID_BYTES: usize = 256;
/// The tail is pinged this often, so an idle connection isn't dropped.
const PING_EVERY: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

const MAX_FRAME: usize = 64 * 1024;
const MAX_ROWS: usize = 256;
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

fn transport() -> EngineError {
    EngineError::Provider {
        evidence: None,
        code: "logs.disconnected".into(),
        message: "The live tail disconnected. Reconnect to observe new events; missed events are not replayed.".into(),
    }
}

fn overflow() -> EngineError {
    EngineError::Provider {
        evidence: None,
        code: "logs.limit".into(),
        message: "The live tail exceeded its bounded event size. Narrow the provider query and reconnect.".into(),
    }
}

fn request(raw: &str) -> Result<tokio_tungstenite::tungstenite::http::Request<()>, EngineError> {
    let url = url::Url::parse(raw).map_err(|_| invalid())?;
    if url.scheme() != "wss"
        || url.host_str() != Some("tail.developers.workers.dev")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.path() == "/"
    {
        return Err(invalid());
    }
    let mut request = raw.into_client_request().map_err(|_| invalid())?;
    request.headers_mut().insert(
        "sec-websocket-protocol",
        "trace-v1".parse().map_err(|_| invalid())?,
    );
    Ok(request)
}

pub(super) async fn open(
    adapter: &Cloudflare,
    token: &Secret,
    account: &str,
    script: &str,
    query: &str,
) -> Result<Box<dyn LogSession>, EngineError> {
    if query.len() > MAX_QUERY_BYTES {
        return Err(EngineError::Invalid(
            "Log queries must be at most 256 bytes.",
        ));
    }
    let scanner = LeakScanner::new().map_err(|_| invalid())?;
    let filters = if query.is_empty() {
        json!([])
    } else {
        json!([{"query":query}])
    };
    let value = adapter
        .call(
            token,
            Method::POST,
            &["accounts", account, "workers", "scripts", script, "tails"],
            &[],
            Some(&json!({"filters":filters})),
        )
        .await?;
    let id = super::text(&value, "id")?.to_owned();
    if id.is_empty() || id.len() > MAX_ID_BYTES {
        return Err(invalid());
    }
    let mut session = Tail {
        adapter: Cloudflare::new(adapter.http.clone()),
        token: Secret::new(token.expose().into()),
        account: account.into(),
        script: script.into(),
        id,
        socket: None,
        closed: false,
        pending: VecDeque::new(),
        scanner,
        next_ping: Instant::now() + PING_EVERY,
    };
    let connected = async {
        let request = request(super::text(&value, "url")?)?;
        session.socket = Some(connect(request).await?);
        Ok::<_, EngineError>(())
    }
    .await;
    if let Err(error) = connected {
        // Delete the tail Cloudflare already made, so it isn't left running
        // behind the connection error.
        session.close().await?;
        return Err(error);
    }
    Ok(Box::new(session))
}

async fn connect(
    request: tokio_tungstenite::tungstenite::http::Request<()>,
) -> Result<Socket, EngineError> {
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME))
        .max_frame_size(Some(MAX_FRAME));
    let (mut socket, response) = timeout(
        CONNECT_TIMEOUT,
        connect_async_with_config(request, Some(config), false),
    )
    .await
    .map_err(|_| transport())?
    .map_err(|_| transport())?;
    if response
        .headers()
        .get("sec-websocket-protocol")
        .and_then(|h| h.to_str().ok())
        != Some("trace-v1")
    {
        return Err(invalid());
    }
    timeout(
        SEND_TIMEOUT,
        socket.send(Message::Text("{\"debug\":false}".into())),
    )
    .await
    .map_err(|_| transport())?
    .map_err(|_| transport())?;
    Ok(socket)
}

struct Tail {
    adapter: Cloudflare,
    token: Secret,
    account: String,
    script: String,
    id: String,
    socket: Option<Socket>,
    closed: bool,
    pending: VecDeque<LogEntry>,
    scanner: LeakScanner,
    next_ping: Instant,
}

#[async_trait::async_trait]
impl LogSession for Tail {
    async fn next(&mut self) -> Result<Option<LogEntry>, EngineError> {
        if let Some(entry) = self.pending.pop_front() {
            return Ok(Some(entry));
        }
        loop {
            let Some(socket) = self.socket.as_mut() else {
                return Ok(None);
            };
            let message = tokio::select! {
                message = socket.next() => message,
                () = tokio::time::sleep_until(self.next_ping) => {
                    timeout(SEND_TIMEOUT, socket.send(Message::Ping(Vec::new().into())))
                        .await.map_err(|_| transport())?.map_err(|_| transport())?;
                    self.next_ping = Instant::now() + PING_EVERY;
                    continue;
                }
            };
            let bytes = match message {
                Some(Ok(Message::Text(text))) => text.as_bytes().to_vec(),
                Some(Ok(Message::Binary(bytes))) => bytes.to_vec(),
                Some(Ok(Message::Close(_))) | None => return Err(transport()),
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {
                    timeout(SEND_TIMEOUT, socket.flush())
                        .await
                        .map_err(|_| transport())?
                        .map_err(|_| transport())?;
                    continue;
                }
                Some(Ok(_)) => continue,
                Some(Err(_)) => return Err(transport()),
            };
            self.pending = decode(&bytes, &self.scanner, &self.token)?;
            if let Some(entry) = self.pending.pop_front() {
                return Ok(Some(entry));
            }
        }
    }

    async fn close(&mut self) -> Result<(), EngineError> {
        self.pending.clear();
        if let Some(mut socket) = self.socket.take() {
            // The tail is still deleted at Cloudflare if the close handshake fails.
            let _ = timeout(CLOSE_TIMEOUT, socket.close(None)).await;
        }
        if !self.closed {
            self.adapter
                .call(
                    &self.token,
                    Method::DELETE,
                    &[
                        "accounts",
                        &self.account,
                        "workers",
                        "scripts",
                        &self.script,
                        "tails",
                        &self.id,
                    ],
                    &[],
                    None,
                )
                .await?;
            self.closed = true;
        }
        Ok(())
    }
}

fn timestamp(value: &Value) -> Option<String> {
    value
        .as_i64()
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|t| t.to_rfc3339())
}

fn sensitive(key: &str) -> bool {
    let key: String = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    key.contains("password")
        || key.contains("secret")
        || key.contains("token")
        || key.contains("credential")
        || matches!(
            key.as_str(),
            "authorization" | "cookie" | "setcookie" | "apikey" | "privatekey" | "headers"
        )
}

pub(super) fn sanitize(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if sensitive(key) {
                    *value = json!("[REDACTED]");
                } else {
                    sanitize(value);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                sanitize(value);
            }
        }
        _ => {}
    }
}

fn rendered(value: &Value) -> String {
    match value.as_str() {
        Some(text) => text.into(),
        None => value.to_string(),
    }
}

fn decode(
    bytes: &[u8],
    scanner: &LeakScanner,
    token: &Secret,
) -> Result<VecDeque<LogEntry>, EngineError> {
    if bytes.len() > MAX_FRAME {
        return Err(overflow());
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let logs = value
        .get("logs")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let exceptions = value
        .get("exceptions")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let outcome = value
        .get("outcome")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    if logs.len() + exceptions.len() + 1 > MAX_ROWS {
        return Err(overflow());
    }
    let observed_at = chrono::Utc::now().to_rfc3339();
    let mut result = VecDeque::new();
    let mut push =
        |level, source: &str, time: &Value, message: String| -> Result<(), EngineError> {
            let message = scanner.redact(&message, std::slice::from_ref(token));
            if message.len() > MAX_MESSAGE_BYTES {
                return Err(overflow());
            }
            result.push_back(LogEntry {
                timestamp: timestamp(time),
                observed_at: observed_at.clone(),
                level,
                source: source.into(),
                message,
            });
            Ok(())
        };
    for log in logs {
        let level = match log.get("level").and_then(Value::as_str) {
            Some("debug") => LogLevel::Debug,
            Some("info" | "log") => LogLevel::Info,
            Some("warn") => LogLevel::Warn,
            Some("error") => LogLevel::Error,
            _ => return Err(invalid()),
        };
        let mut messages = log
            .get("message")
            .filter(|v| v.is_array())
            .cloned()
            .ok_or_else(invalid)?;
        sanitize(&mut messages);
        let message = messages
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .map(rendered)
            .collect::<Vec<_>>()
            .join(" ");
        push(level, "console", &log["timestamp"], message)?;
    }
    for exception in exceptions {
        let mut message = exception.get("message").cloned().ok_or_else(invalid)?;
        sanitize(&mut message);
        // Stacks and request metadata are left out on purpose.
        push(
            LogLevel::Error,
            "exception",
            &exception["timestamp"],
            rendered(&message),
        )?;
    }
    // Only the outcome, never request headers, URLs, client IPs or bodies.
    push(
        if outcome == "ok" {
            LogLevel::Info
        } else {
            LogLevel::Error
        },
        "invocation",
        &value["eventTimestamp"],
        format!("Invocation outcome: {outcome}"),
    )?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_origin_is_exact_and_never_forwards_authorization() {
        let request = request("wss://tail.developers.workers.dev/opaque").unwrap();
        assert_eq!(request.headers()["sec-websocket-protocol"], "trace-v1");
        assert!(!request.headers().contains_key("authorization"));
        for url in [
            "ws://tail.developers.workers.dev/id",
            "wss://tail.developers.workers.dev.evil.test/id",
            "wss://user@tail.developers.workers.dev/id",
            "wss://tail.developers.workers.dev:8443/id",
            "wss://tail.developers.workers.dev/id#fragment",
            "wss://localhost/id",
        ] {
            assert!(super::request(url).is_err(), "{url}");
        }
    }

    #[test]
    fn decode_removes_metadata_and_nested_secrets_without_fabricating_times() {
        let scanner = LeakScanner::new().unwrap();
        let token = Secret::new("provider-credential".into());
        let event = json!({"outcome":"ok", "eventTimestamp":1700000000000_i64,
            "event":{"request":{"url":"https://example.test/?password=private", "headers":{"cookie":"private"}, "cf":{"ip":"private"}}},
            "logs":[{"level":"log","message":["provider-credential", {"nested":[{"access_token":"private", "password":"private", "value":42}]}]}],
            "exceptions":[{"message":"failure", "stack":"private"}]});
        let rows = decode(event.to_string().as_bytes(), &scanner, &token).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows[0].timestamp.is_none());
        assert_eq!(
            rows[2].timestamp.as_deref(),
            Some("2023-11-14T22:13:20+00:00")
        );
        let text = serde_json::to_string(&rows).unwrap();
        assert!(!text.contains("private"));
        assert!(!text.contains("provider-credential"));
        assert!(text.contains("42"));
    }

    #[test]
    fn malformed_and_oversized_events_fail_explicitly() {
        let scanner = LeakScanner::new().unwrap();
        let token = Secret::new(String::new());
        for value in [
            json!({}),
            json!({"outcome":"ok","exceptions":[],"logs":[{"level":"unknown","message":[]}]}),
            json!({
                "outcome": "ok",
                "exceptions": [],
                "logs": [{"level": "info", "message": ["x".repeat(MAX_MESSAGE_BYTES+1)]}],
            }),
        ] {
            assert!(decode(value.to_string().as_bytes(), &scanner, &token).is_err());
        }
        assert!(decode(&vec![b' '; MAX_FRAME + 1], &scanner, &token).is_err());
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use crate::http::ProviderHttp;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[tokio::test]
    async fn invalid_tail_origin_is_deleted_after_creation() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for method in ["POST", "DELETE"] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![];
                loop {
                    let mut bytes = [0; 2048];
                    let count = socket.read(&mut bytes).await.unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&bytes[..count]);
                    if request.windows(4).any(|s| s == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&request);
                let path = "/accounts/account/workers/scripts/script/tails";
                assert!(request.starts_with(&format!(
                    "{method} {path}{} HTTP/1.1",
                    if method == "DELETE" { "/tail-id" } else { "" }
                )));
                let body = if method == "POST" {
                    r#"{"result":{"id":"tail-id","url":"wss://evil.example/private"}}"#
                } else {
                    r#"{"result":null}"#
                };
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        });
        let adapter = Cloudflare::new(ProviderHttp::local_fixture(
            url::Url::parse(&format!("http://{address}/")).unwrap(),
        ));
        assert!(
            open(
                &adapter,
                &Secret::new("test-token".into()),
                "account",
                "script",
                ""
            )
            .await
            .is_err()
        );
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    #[allow(clippy::result_large_err)] // Tungstenite fixes the handshake callback error type.
    async fn websocket_negotiates_trace_v1_and_preserves_events_across_cancelled_reads() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (release, wait) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut websocket = tokio_tungstenite::accept_hdr_async(socket, |request: &tokio_tungstenite::tungstenite::handshake::server::Request, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                assert_eq!(request.headers()["sec-websocket-protocol"], "trace-v1");
                assert!(!request.headers().contains_key("authorization"));
                response.headers_mut().insert("sec-websocket-protocol", "trace-v1".parse().unwrap());
                Ok(response)
            }).await.unwrap();
            assert_eq!(
                websocket
                    .next()
                    .await
                    .unwrap()
                    .unwrap()
                    .into_text()
                    .unwrap(),
                "{\"debug\":false}"
            );
            wait.await.unwrap();
            websocket
                .send(Message::Text(
                    json!({
                        "outcome": "ok",
                        "eventTimestamp": null,
                        "exceptions": [],
                        "logs": [
                            {"level": "info", "message": ["first"]},
                            {"level": "warn", "message": ["second"]},
                        ],
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            while let Some(Ok(message)) = websocket.next().await {
                if matches!(message, Message::Close(_)) {
                    break;
                }
            }
        });
        // Only this fixture skips `request`, which accepts nothing but Cloudflare's
        // tail address.
        let mut request = format!("ws://{address}/tail")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("sec-websocket-protocol", "trace-v1".parse().unwrap());
        let socket = connect(request).await.unwrap();
        let mut tail = Tail {
            adapter: Cloudflare::new(ProviderHttp::new().unwrap()),
            token: Secret::new("test-token".into()),
            account: "account".into(),
            script: "script".into(),
            id: "tail".into(),
            socket: Some(socket),
            closed: true,
            pending: VecDeque::new(),
            scanner: LeakScanner::new().unwrap(),
            next_ping: Instant::now() + PING_EVERY,
        };
        assert!(
            timeout(Duration::from_millis(10), tail.next())
                .await
                .is_err()
        );
        release.send(()).unwrap();
        assert_eq!(tail.next().await.unwrap().unwrap().message, "first");
        assert_eq!(tail.next().await.unwrap().unwrap().message, "second");
        assert_eq!(tail.next().await.unwrap().unwrap().source, "invocation");
        tail.close().await.unwrap();
        assert!(tail.next().await.unwrap().is_none());
        timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
    }
}
