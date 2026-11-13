//! One-off addresses on 127.0.0.1 that the browser comes back to after a
//! sign-in, as desktop apps do (RFC 8252). The caller gets each visit to
//! the address's path and decides what page to answer with; any other path
//! gets a 404.

use crate::error::{Error, Result};
use bytes::Bytes;
use http_body_util::Full;
use hyper::service::service_fn;
use hyper::{Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use tauri::async_runtime::JoinHandle;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};

/// The page the browser shows once it's back.
pub struct Page {
    pub ok: bool,
    pub html: String,
}

impl Page {
    pub fn new(ok: bool, title: &str, text: &str) -> Self {
        let html = include_str!("../pages/done.html")
            .replace("{{title}}", &escape(title))
            .replace("{{text}}", &escape(text));
        Self { ok, html }
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// One visit to the address, waiting for its page.
pub struct Visit {
    query: HashMap<String, String>,
    reply: oneshot::Sender<Page>,
}

impl Visit {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.query.get(name).map(String::as_str)
    }

    pub fn answer(self, page: Page) {
        // The browser may have gone; then there's no one to show it to.
        let _gone = self.reply.send(page);
    }
}

/// A listening address. It stops listening when dropped.
pub struct Listener {
    pub port: u16,
    visits: mpsc::Receiver<Visit>,
    task: JoinHandle<()>,
}

impl Listener {
    /// Listens on `port` (0 picks a free one) for visits to `path`.
    pub async fn bind(port: u16, path: &'static str) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port)).await.map_err(|_| {
            Error::new(
                "sign_in.port",
                "Another sign-in is waiting on this computer. Finish or close it, then try again.",
            )
        })?;
        let port = listener.local_addr()?.port();
        let (sender, visits) = mpsc::channel(4);
        let task = tauri::async_runtime::spawn(accept(listener, path, sender));
        Ok(Self { port, visits, task })
    }

    pub fn address(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    pub async fn next(&mut self) -> Option<Visit> {
        self.visits.recv().await
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn accept(listener: TcpListener, path: &'static str, visits: mpsc::Sender<Visit>) {
    while let Ok((stream, _)) = listener.accept().await {
        let visits = visits.clone();
        let service = service_fn(move |request| answer(request, path, visits.clone()));
        tokio::spawn(
            hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service),
        );
    }
}

async fn answer(
    request: hyper::Request<hyper::body::Incoming>,
    path: &str,
    visits: mpsc::Sender<Visit>,
) -> Result<Response<Full<Bytes>>, std::convert::Infallible> {
    if request.uri().path() != path {
        return Ok(status_only(StatusCode::NOT_FOUND));
    }
    let query = url::form_urlencoded::parse(request.uri().query().unwrap_or("").as_bytes())
        .into_owned()
        .collect();
    let (reply, page) = oneshot::channel();
    if visits.send(Visit { query, reply }).await.is_err() {
        return Ok(status_only(StatusCode::NOT_FOUND));
    }
    let Ok(page) = page.await else {
        return Ok(status_only(StatusCode::NOT_FOUND));
    };
    let status = if page.ok {
        StatusCode::OK
    } else {
        StatusCode::BAD_REQUEST
    };
    Ok(Response::builder()
        .status(status)
        .header("content-type", "text/html; charset=utf-8")
        .header("cache-control", "no-store")
        .body(Full::new(Bytes::from(page.html)))
        .expect("static response parts"))
}

fn status_only(status: StatusCode) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::new()));
    *response.status_mut() = status;
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn get(port: u16, target: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let request =
            format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        response
    }

    #[tokio::test]
    async fn hands_over_a_visit_and_shows_its_page() {
        let mut listener = Listener::bind(0, "/done").await.unwrap();
        let port = listener.port;
        let browser = tokio::spawn(async move { get(port, "/done?code=a%20b&state=xyz").await });
        let visit = listener.next().await.unwrap();
        assert_eq!(visit.get("code"), Some("a b"));
        assert_eq!(visit.get("state"), Some("xyz"));
        visit.answer(Page::new(true, "Signed in", "<you>"));
        let response = browser.await.unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("&lt;you&gt;"));
    }

    #[tokio::test]
    async fn other_paths_are_not_found() {
        let listener = Listener::bind(0, "/done").await.unwrap();
        assert!(
            get(listener.port, "/other")
                .await
                .starts_with("HTTP/1.1 404")
        );
    }
}
