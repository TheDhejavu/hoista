//! Bounded live-log SSE decoding over the existing owner-only IPC transport.

use super::{
    client::{ClientError, LocalClient},
    contracts::ErrorDto,
};
use crate::application::engine::LogFrame;
use axum::http::{Method, Request};
use http_body_util::{BodyExt, Full, Limited};
use hyper_util::rt::TokioIo;
use std::time::Duration;

/// The largest log event, or error answer, the stream accepts.
const MAX_FRAME_BYTES: usize = 64 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the stream may go without a frame. The engine sends a keep-alive well within it.
const QUIET_LIMIT: Duration = Duration::from_secs(60);

impl LocalClient {
    /// Reads a live tail until its terminal frame; dropping this future disconnects the viewer.
    /// The callback receives coverage and termination frames as well as log records.
    ///
    /// # Errors
    /// Reports IPC failure, malformed/oversized events, missing terminal frames or API rejection.
    pub async fn follow_logs<F>(&self, path: &str, mut receive: F) -> Result<(), ClientError>
    where
        F: FnMut(LogFrame) -> Result<(), ClientError>,
    {
        if !path.starts_with('/') || path.starts_with("//") {
            return Err(ClientError::Transport);
        }
        if self.server.is_some() {
            return Err(ClientError::Rejected(
                "Live logs work on this computer only for now. Open them in your provider's dashboard.".into(),
            ));
        }
        let stream = tokio::time::timeout(CONNECT_TIMEOUT, super::transport::connect(&self.paths))
            .await
            .map_err(|_| ClientError::Transport)?
            .map_err(|_| ClientError::Transport)?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|_| ClientError::Transport)?;
        tokio::pin!(connection);
        let read = async {
            let request = Request::builder()
                .method(Method::GET)
                .uri(path)
                .header("host", "localhost")
                .header("accept", "text/event-stream")
                .body(Full::new(axum::body::Bytes::new()))
                .map_err(|_| ClientError::Transport)?;
            let response = tokio::time::timeout(QUIET_LIMIT, sender.send_request(request))
                .await
                .map_err(|_| ClientError::Transport)?
                .map_err(|_| ClientError::Transport)?;
            if !response.status().is_success() {
                let bytes = tokio::time::timeout(
                    CONNECT_TIMEOUT,
                    Limited::new(response.into_body(), MAX_FRAME_BYTES).collect(),
                )
                .await
                .map_err(|_| ClientError::Transport)?
                .map_err(|_| ClientError::Transport)?
                .to_bytes();
                let error: ErrorDto =
                    serde_json::from_slice(&bytes).map_err(|_| ClientError::Transport)?;
                return Err(ClientError::Rejected(error.explanation()));
            }
            if response
                .headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok())
                .is_none_or(|s| s.split(';').next().map(str::trim) != Some("text/event-stream"))
            {
                return Err(ClientError::Transport);
            }
            let mut body = response.into_body();
            let mut decoder = Decoder::default();
            loop {
                let frame = tokio::time::timeout(QUIET_LIMIT, body.frame())
                    .await
                    .map_err(|_| ClientError::Transport)?
                    .ok_or(ClientError::Transport)?
                    .map_err(|_| ClientError::Transport)?;
                if let Ok(bytes) = frame.into_data() {
                    for byte in bytes {
                        if let Some(frame) = decoder.byte(byte)? {
                            let terminal = matches!(frame, LogFrame::End { .. });
                            receive(frame)?;
                            if terminal {
                                return Ok(());
                            }
                        }
                    }
                }
            }
        };
        tokio::select! {
            biased;
            result = read => result,
            _ = &mut connection => Err(ClientError::Transport),
        }
    }
}

#[derive(Default)]
struct Decoder {
    line: Vec<u8>,
    data: String,
    event: String,
    size: usize,
}

impl Decoder {
    fn byte(&mut self, byte: u8) -> Result<Option<LogFrame>, ClientError> {
        self.size += 1;
        if self.size > MAX_FRAME_BYTES {
            return Err(ClientError::Transport);
        }
        if byte != b'\n' {
            self.line.push(byte);
            return Ok(None);
        }
        if self.line.last() == Some(&b'\r') {
            self.line.pop();
        }
        let line = std::str::from_utf8(&self.line).map_err(|_| ClientError::Transport)?;
        if line.is_empty() {
            self.size = 0;
            self.line.clear();
            if self.data.is_empty() {
                self.event.clear();
                return Ok(None);
            }
            let frame: LogFrame =
                serde_json::from_str(&self.data).map_err(|_| ClientError::Transport)?;
            let name = match &frame {
                LogFrame::Ready { .. } => "ready",
                LogFrame::Log { .. } => "log",
                LogFrame::End { .. } => "end",
            };
            if self.event != name {
                return Err(ClientError::Transport);
            }
            self.data.clear();
            self.event.clear();
            return Ok(Some(frame));
        }
        if !line.starts_with(':') {
            let (field, value) = line.split_once(':').unwrap_or((line, ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "event" => self.event = value.into(),
                "data" => {
                    if !self.data.is_empty() {
                        self.data.push('\n');
                    }
                    self.data.push_str(value);
                }
                _ => {}
            }
        }
        self.line.clear();
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_crlf_multiline_and_keepalive_frames_are_bounded_and_typed() {
        let input = b":keepalive\r\n\r\nevent: ready\r\ndata: {\"type\":\"ready\",\r\ndata: \"started_at\":\"now\",\"historical\":false}\r\n\r\nevent: end\ndata: {\"type\":\"end\",\"reason\":\"logs.closed\",\"cleanup_complete\":true}\n\n";
        let mut decoder = Decoder::default();
        let mut frames = vec![];
        for byte in input {
            if let Some(frame) = decoder.byte(*byte).unwrap() {
                frames.push(frame);
            }
        }
        assert_eq!(frames.len(), 2);
        assert!(matches!(
            frames[0],
            LogFrame::Ready {
                historical: false,
                ..
            }
        ));
        assert!(matches!(
            frames[1],
            LogFrame::End {
                cleanup_complete: true,
                ..
            }
        ));
        let mut decoder = Decoder::default();
        assert!((0..=64 * 1024).any(|_| decoder.byte(b'x').is_err()));
        for input in [
            b"event: log\ndata: {\"type\":\"end\",\"reason\":\"x\",\"cleanup_complete\":true}\n\n"
                .as_slice(),
            b"event: log\ndata: broken\n\n",
            b"data: \xff\n\n",
        ] {
            let mut decoder = Decoder::default();
            assert!(input.iter().any(|byte| decoder.byte(*byte).is_err()));
        }
    }
}

#[cfg(test)]
mod ipc_tests {
    use super::*;
    use crate::infrastructure::store::SqliteStore;
    use axum::{Router, body::Body, http::Response, routing::get};
    use hoista_system::paths::StatePaths;

    #[tokio::test]
    async fn local_stream_delivers_terminal_frames_and_rejects_truncation() {
        let directory = tempfile::tempdir().unwrap();
        let paths = StatePaths::new(Some(directory.path().join("s"))).unwrap();
        let store = SqliteStore::open(&paths).await.unwrap();
        let listener = crate::transport::LocalListener::bind(&paths).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(paths.root())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(paths.socket())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        let ready = "event: ready\ndata: {\"type\":\"ready\",\"started_at\":\"now\",\"historical\":false}\n\n";
        let complete = format!(
            "{ready}event: end\ndata: {{\"type\":\"end\",\"reason\":\"logs.session_expired\",\"cleanup_complete\":true}}\n\n"
        );
        let api = Router::new()
            .route(
                "/complete",
                get(move || {
                    let bytes = complete.clone();
                    async move {
                        let chunks: Vec<_> = bytes
                            .as_bytes()
                            .chunks(3)
                            .map(|bytes| {
                                Ok::<_, std::convert::Infallible>(
                                    axum::body::Bytes::copy_from_slice(bytes),
                                )
                            })
                            .collect();
                        Response::builder()
                            .header("content-type", "text/event-stream; charset=utf-8")
                            .body(Body::from_stream(futures_util::stream::iter(chunks)))
                            .unwrap()
                    }
                }),
            )
            .route(
                "/truncated",
                get(move || async move {
                    Response::builder()
                        .header("content-type", "text/event-stream")
                        .body(Body::from(ready))
                        .unwrap()
                }),
            )
            .route(
                "/rejected",
                get(|| async {
                    (
                        axum::http::StatusCode::FORBIDDEN,
                        axum::Json(serde_json::json!({"code":"denied","message":"Access denied."})),
                    )
                }),
            );
        let shutdown = tokio_util::sync::CancellationToken::new();
        let stop = shutdown.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, api)
                .with_graceful_shutdown(stop.cancelled_owned())
                .await
                .unwrap();
        });
        let client = LocalClient::new(&paths).unwrap();
        let mut frames = vec![];
        client
            .follow_logs("/complete", |frame| {
                frames.push(frame);
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(frames.len(), 2);
        assert!(matches!(&frames[1],LogFrame::End { reason,.. } if reason=="logs.session_expired"));
        assert!(client.follow_logs("/truncated", |_| Ok(())).await.is_err());
        assert!(
            matches!(client.follow_logs("/rejected",|_|Ok(())).await,Err(ClientError::Rejected(message)) if message=="[denied] Access denied.")
        );
        assert!(
            client
                .follow_logs("/complete", |_| Err(ClientError::Rejected(
                    "output closed".into()
                )))
                .await
                .is_err()
        );
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
        store.close().await;
    }
}
