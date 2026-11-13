//! Railway's live updates: GraphQL subscriptions over a WebSocket, using the
//! `graphql-transport-ws` protocol Railway's own CLI speaks. Only Railway's
//! API host is ever dialled, frames have a size limit, and the token goes only in
//! the upgrade request's Authorization header.

use crate::{application::ports::catalog::EngineError, domain::Secret};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::{net::TcpStream, time::timeout};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async_with_config,
    tungstenite::{
        client::IntoClientRequest,
        protocol::{Message, WebSocketConfig},
    },
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

const ENDPOINT: &str = "wss://backboard.railway.com/graphql/v2";
const PROTOCOL: &str = "graphql-transport-ws";
const MAX_FRAME: usize = 1024 * 1024;
/// Railway pings; if nothing at all arrives for this long, the socket is gone.
const QUIET_LIMIT: Duration = Duration::from_secs(90);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

fn lost() -> EngineError {
    EngineError::Provider {
        evidence: None,
        code: "provider.live_lost".into(),
        message:
            "Railway's live connection dropped. Hoista falls back to reading every few seconds."
                .into(),
    }
}

/// One subscription on its own connection.
pub(super) struct Subscription {
    socket: Socket,
}

impl Subscription {
    /// Connects and starts `query` (a `subscription …` document).
    ///
    /// # Errors
    /// The connection or Railway's acknowledgement failing, or Railway refusing the subscription.
    pub(super) async fn start(
        token: &Secret,
        query: &str,
        variables: Value,
    ) -> Result<Self, EngineError> {
        // The credential keeper signs HTTP requests, not WebSockets, so a token
        // handle (Hoista on a server) can't open one.
        if token.handle_key().is_some() {
            return Err(EngineError::Provider {
                evidence: None,
                code: "logs.not_on_server".into(),
                message: "Live log tails from Hoista on a server aren't available yet. Earlier logs work; for a live tail, open the logs in Railway.".into(),
            });
        }
        let mut request = ENDPOINT.into_client_request().map_err(|_| lost())?;
        let headers = request.headers_mut();
        headers.insert(
            "sec-websocket-protocol",
            PROTOCOL.parse().map_err(|_| lost())?,
        );
        headers.insert(
            "authorization",
            format!("Bearer {}", token.expose())
                .parse()
                .map_err(|_| lost())?,
        );
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_FRAME))
            .max_frame_size(Some(MAX_FRAME));
        let (mut socket, response) = timeout(
            CONNECT_TIMEOUT,
            connect_async_with_config(request, Some(config), false),
        )
        .await
        .map_err(|_| lost())?
        .map_err(|_| lost())?;
        if response
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|value| value.to_str().ok())
            != Some(PROTOCOL)
        {
            return Err(lost());
        }
        send(
            &mut socket,
            &json!({"type": "connection_init", "payload": {}}),
        )
        .await?;
        // Wait for the acknowledgement, answering pings meanwhile.
        loop {
            match receive(&mut socket).await? {
                Frame::Ack => break,
                Frame::Ping => send(&mut socket, &json!({"type": "pong"})).await?,
                _ => {}
            }
        }
        send(
            &mut socket,
            &json!({"id": "1", "type": "subscribe", "payload": {"query": query, "variables": variables}}),
        )
        .await?;
        Ok(Self { socket })
    }

    /// The next answer's `data`, or None when Railway ends the subscription.
    ///
    /// # Errors
    /// The connection dropping or Railway reporting an error for the subscription.
    pub(super) async fn next(&mut self) -> Result<Option<Value>, EngineError> {
        loop {
            match receive(&mut self.socket).await? {
                Frame::Next(data) => return Ok(Some(data)),
                Frame::Complete => return Ok(None),
                Frame::Error => return Err(lost()),
                Frame::Ping => send(&mut self.socket, &json!({"type": "pong"})).await?,
                Frame::Ack | Frame::Other => {}
            }
        }
    }

    /// Ends the subscription and closes the connection.
    pub(super) async fn close(&mut self) {
        let _ = send(&mut self.socket, &json!({"id": "1", "type": "complete"})).await;
        let _ = timeout(CLOSE_TIMEOUT, self.socket.close(None)).await;
    }
}

enum Frame {
    Ack,
    Ping,
    Next(Value),
    Error,
    Complete,
    Other,
}

async fn send(socket: &mut Socket, value: &Value) -> Result<(), EngineError> {
    timeout(
        SEND_TIMEOUT,
        socket.send(Message::Text(value.to_string().into())),
    )
    .await
    .map_err(|_| lost())?
    .map_err(|_| lost())
}

async fn receive(socket: &mut Socket) -> Result<Frame, EngineError> {
    loop {
        let message = timeout(QUIET_LIMIT, socket.next())
            .await
            .map_err(|_| lost())?;
        let text = match message {
            Some(Ok(Message::Text(text))) => text.to_string(),
            Some(Ok(
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) | Message::Binary(_),
            )) => {
                continue;
            }
            Some(Ok(Message::Close(_)) | Err(_)) | None => return Err(lost()),
        };
        let value: Value = serde_json::from_str(&text).map_err(|_| lost())?;
        return Ok(match value["type"].as_str() {
            Some("connection_ack") => Frame::Ack,
            Some("ping") => Frame::Ping,
            Some("next") => {
                if value["payload"]["errors"]
                    .as_array()
                    .is_some_and(|errors| !errors.is_empty())
                    && value["payload"]["data"].is_null()
                {
                    Frame::Error
                } else {
                    Frame::Next(value["payload"]["data"].clone())
                }
            }
            Some("error") => Frame::Error,
            Some("complete") => Frame::Complete,
            _ => Frame::Other,
        });
    }
}
