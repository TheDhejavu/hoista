//! A one-off address on this computer that a browser comes back to after
//! signing in somewhere (RFC 8252): it waits for one request, answers with
//! a short page, and hands back the query string.

use anyhow::{Context as _, bail};
use std::collections::HashMap;
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

/// The most of the browser's return request that's read.
const MAX_REQUEST_BYTES: usize = 8192;
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Listening on `127.0.0.1`, on `port` or any free one when it's 0.
pub(crate) struct Loopback {
    listener: TcpListener,
}

impl Loopback {
    pub(crate) async fn bind(port: u16) -> anyhow::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .await
            .with_context(|| {
                format!("Couldn't listen on 127.0.0.1:{port}. Is another sign-in open?")
            })?;
        Ok(Self { listener })
    }

    pub(crate) fn port(&self) -> anyhow::Result<u16> {
        Ok(self.listener.local_addr()?.port())
    }

    /// Waits for the browser to come back to `path`, up to `within`.
    /// `answer` gets the query and returns whether it worked and what the
    /// page should say.
    pub(crate) async fn wait<F, Fut>(
        self,
        path: &str,
        within: Duration,
        mut answer: F,
    ) -> anyhow::Result<HashMap<String, String>>
    where
        F: FnMut(HashMap<String, String>) -> Fut,
        Fut: std::future::Future<Output = (bool, String)>,
    {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let accepted = tokio::time::timeout_at(deadline, self.listener.accept()).await;
            let Ok(accepted) = accepted else {
                bail!(
                    "Nothing came back within {} minutes. Try again.",
                    within.as_secs() / 60
                );
            };
            let (mut stream, _) = accepted?;
            let mut buffer = vec![0u8; MAX_REQUEST_BYTES];
            let read = tokio::time::timeout(READ_TIMEOUT, stream.read(&mut buffer))
                .await
                .unwrap_or(Ok(0))
                .unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]);
            let target = request
                .lines()
                .next()
                .and_then(|line| line.strip_prefix("GET "))
                .and_then(|rest| rest.split(' ').next())
                .unwrap_or_default();
            let (at, query) = target.split_once('?').unwrap_or((target, ""));
            if at != path {
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                    )
                    .await;
                continue;
            }
            let params: HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes())
                .into_owned()
                .collect();
            let (ok, message) = answer(params.clone()).await;
            let page = page(ok, &message);
            let head = format!(
                "HTTP/1.1 {}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n",
                if ok { "200 OK" } else { "400 Bad Request" },
                page.len()
            );
            let _ = stream.write_all(head.as_bytes()).await;
            let _ = stream.write_all(page.as_bytes()).await;
            let _ = stream.shutdown().await;
            return Ok(params);
        }
    }
}

fn page(ok: bool, message: &str) -> String {
    hoista_server::views::result_page(if ok { "Done" } else { "That didn't work" }, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn it_answers_one_request_at_its_path_and_ignores_others() {
        let loopback = Loopback::bind(0).await.unwrap();
        let port = loopback.port().unwrap();
        let waiting = tokio::spawn(async move {
            loopback
                .wait("/done", Duration::from_secs(5), |params| async move {
                    (params.contains_key("code"), "<b>back</b>".to_owned())
                })
                .await
        });
        let send = |target: &'static str| async move {
            let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            stream
                .write_all(format!("GET {target} HTTP/1.1\r\nhost: x\r\n\r\n").as_bytes())
                .await
                .unwrap();
            let mut reply = String::new();
            stream.read_to_string(&mut reply).await.unwrap();
            reply
        };
        assert!(send("/favicon.ico").await.starts_with("HTTP/1.1 404"));
        let reply = send("/done?code=abc&state=s%201").await;
        assert!(reply.starts_with("HTTP/1.1 200"));
        assert!(reply.contains("back") && !reply.contains("<b>back"));
        let params = waiting.await.unwrap().unwrap();
        assert_eq!(params["code"], "abc");
        assert_eq!(params["state"], "s 1");
    }
}
