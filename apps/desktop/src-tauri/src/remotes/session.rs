use super::{Remote, relay};
use crate::engine::{Reply, Request, Splitter};
use crate::error::{Error, Result};
use futures_util::StreamExt;
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// A remote this computer is signed in to: its `/api`, with this computer's key.
#[derive(Clone)]
pub struct Session {
    pub remote: Remote,
    key: String,
    http: reqwest::Client,
}

impl Session {
    pub fn new(remote: Remote, key: String, http: reqwest::Client) -> Self {
        Self { remote, key, http }
    }

    /// A request from the screens. Connecting a provider account is relayed
    /// through this computer, since the provider sends the browser back here.
    pub async fn request(&self, request: &Request) -> Result<Reply> {
        if request.method == "POST" && request.path == "/accounts/sign-in" {
            return relay::start(self, request).await;
        }
        self.send(request).await
    }

    /// A request straight to the remote's API.
    pub async fn send(&self, request: &Request) -> Result<Reply> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| Error::new("request.invalid", "Not a request the engine takes."))?;
        let mut builder = self
            .http
            .request(method, self.api(&request.path))
            .bearer_auth(&self.key)
            .timeout(REQUEST_TIMEOUT);
        if let Some(body) = &request.body {
            builder = builder.json(body);
        }
        let response = builder.send().await.map_err(|_| {
            Error::new(
                "server.unreachable",
                "Couldn't reach your remote Hoista. If you were changing something, check it there before trying again.",
            )
        })?;
        if response.status().is_redirection() {
            return Err(Error::new(
                "server.refused",
                "Your remote Hoista sent the request somewhere else. Nothing was changed.",
            ));
        }
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        Ok(Reply { status, text })
    }

    /// Follows the remote's live events until the stream drops.
    pub async fn events(&self, mut on_message: impl FnMut(String) -> bool) -> Result<()> {
        let response = self
            .http
            .get(self.api("/events"))
            .bearer_auth(&self.key)
            .header("accept", "text/event-stream")
            .send()
            .await
            .map_err(|_| Error::new("server.unreachable", "Couldn't reach your remote Hoista."))?;
        if !response.status().is_success() {
            return Err(Error::new(
                "stream.refused",
                "Your remote Hoista refused the event stream.",
            ));
        }
        let mut chunks = response.bytes_stream();
        let mut splitter = Splitter::default();
        while let Some(Ok(chunk)) = chunks.next().await {
            let Some(messages) = splitter.push(&chunk) else {
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

    fn api(&self, path: &str) -> String {
        format!("{}/api{path}", self.remote.url)
    }
}
