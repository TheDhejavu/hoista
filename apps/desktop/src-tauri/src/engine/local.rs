use super::{Reply, Request, Splitter};
use crate::error::{Error, Result};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use std::path::PathBuf;

/// This computer's `hoistd`: HTTP and JSON over its socket (a named pipe on
/// Windows), the same API the CLI uses.
#[derive(Debug, Clone)]
pub struct Local {
    socket: PathBuf,
}

impl Local {
    pub fn new(socket: PathBuf) -> Self {
        Self { socket }
    }

    pub async fn request(&self, request: &Request) -> Result<Reply> {
        request.check()?;
        let body = request
            .body
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let response = self
            .send(&request.method, &request.path, body, None)
            .await?;
        let status = response.status().as_u16();
        let bytes = response
            .into_body()
            .collect()
            .await
            .map_err(|_| unreachable())?
            .to_bytes();
        Ok(Reply {
            status,
            text: String::from_utf8_lossy(&bytes).into_owned(),
        })
    }

    /// Follows a server-sent stream such as `/events`, calling `on_message`
    /// with each message until the engine closes it or `on_message` returns
    /// false.
    pub async fn stream(
        &self,
        path: &str,
        last_event_id: Option<&str>,
        mut on_message: impl FnMut(String) -> bool,
    ) -> Result<()> {
        let response = self.send("GET", path, None, last_event_id).await?;
        if !response.status().is_success() {
            return Err(Error::new(
                "stream.refused",
                format!("The engine refused the stream ({}).", response.status()),
            ));
        }
        let mut body = response.into_body();
        let mut splitter = Splitter::default();
        while let Some(Ok(frame)) = body.frame().await {
            let Some(chunk) = frame.data_ref() else {
                continue;
            };
            let Some(messages) = splitter.push(chunk) else {
                break;
            };
            for message in messages {
                if !on_message(message) {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    async fn send(
        &self,
        method: &str,
        path: &str,
        body: Option<String>,
        last_event_id: Option<&str>,
    ) -> Result<hyper::Response<Incoming>> {
        let request = build(method, path, body, last_event_id)?;
        let stream = connect(&self.socket).await.map_err(|_| unreachable())?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|_| unreachable())?;
        tokio::spawn(connection);
        sender
            .send_request(request)
            .await
            .map_err(|_| unreachable())
    }
}

fn unreachable() -> Error {
    Error::new("engine_unreachable", "The Hoista engine is not running.")
}

fn build(
    method: &str,
    path: &str,
    body: Option<String>,
    last_event_id: Option<&str>,
) -> Result<hyper::Request<Full<Bytes>>> {
    let invalid = || Error::new("request.invalid", "Not a request the engine takes.");
    let method = hyper::Method::from_bytes(method.as_bytes()).map_err(|_| invalid())?;
    let mut request = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header(hyper::header::HOST, "hoistd");
    if let Some(id) = last_event_id {
        request = request.header("last-event-id", id);
    }
    if body.is_some() {
        request = request.header(hyper::header::CONTENT_TYPE, "application/json");
    }
    request
        .body(Full::new(body.map(Bytes::from).unwrap_or_default()))
        .map_err(|_| invalid())
}

#[cfg(unix)]
async fn connect(socket: &std::path::Path) -> std::io::Result<tokio::net::UnixStream> {
    tokio::net::UnixStream::connect(socket).await
}

#[cfg(windows)]
async fn connect(
    socket: &std::path::Path,
) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    tokio::net::windows::named_pipe::ClientOptions::new().open(socket)
}
