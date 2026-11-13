//! Connecting a provider account while a remote is shown. The provider sends
//! the browser back to this computer, which the remote can't reach, so the
//! remote starts the sign-in without listening, and this computer listens on
//! the same port and passes on what comes back. The remote keeps the PKCE
//! secret and trades the code itself.

use super::Session;
use crate::engine::{Reply, Request};
use crate::error::Result;
use crate::loopback::{Listener, Page};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

/// The provider's page gives up after five minutes; so does the relay.
const GIVE_UP_AFTER: Duration = Duration::from_secs(5 * 60);

#[derive(Deserialize)]
struct Started {
    id: String,
    relay_port: Option<u16>,
}

pub async fn start(session: &Session, request: &Request) -> Result<Reply> {
    let mut body = request.body.clone().unwrap_or_else(|| json!({}));
    if let Value::Object(fields) = &mut body {
        fields.insert("relay".to_owned(), Value::Bool(true));
    }
    let asked = Request::new("POST", "/accounts/sign-in", Some(body));
    let reply = session.send(&asked).await?;
    if reply.failed() {
        return Err(reply.error());
    }
    let Ok(started) = serde_json::from_str::<Started>(&reply.text) else {
        return Ok(reply);
    };
    let Some(port) = started.relay_port else {
        return Ok(reply);
    };
    let listener = Listener::bind(port, "/callback").await?;
    let session = session.clone();
    tauri::async_runtime::spawn(async move {
        // Nothing came back in time: dropping the listener frees the port.
        let _gave_up =
            tokio::time::timeout(GIVE_UP_AFTER, pass_on(listener, &session, &started.id)).await;
    });
    Ok(reply)
}

/// Waits for the provider to send the browser back, and forwards it.
async fn pass_on(mut listener: Listener, session: &Session, sign_in: &str) {
    let Some(visit) = listener.next().await else {
        return;
    };
    let code = visit.get("code").map(str::to_owned);
    let mut body = json!({ "state": visit.get("state").unwrap_or_default() });
    if let Some(code) = &code {
        body["code"] = json!(code);
    }
    let path = format!("/accounts/sign-in/{}/callback", super::segment(sign_in));
    let forwarded = session.send(&Request::new("POST", path, Some(body))).await;
    let remote = &session.remote.name;
    let page = match forwarded.and_then(|reply| reply.data::<Value>()) {
        Ok(_) if code.is_some() => Page::new(
            true,
            "Account connected",
            &format!("Go back to Hoista. It’s connected to {remote}. You can close this tab."),
        ),
        Ok(_) => Page::new(
            false,
            "Account not connected",
            "Nothing was saved. Go back to Hoista to try again.",
        ),
        Err(error) => Page::new(false, "Account not connected", &error.message),
    };
    visit.answer(page);
}
