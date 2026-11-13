//! Signing this computer in to a remote, the way desktop apps sign in
//! elsewhere (OAuth with PKCE and a loopback address, RFC 8252). The
//! remote's page opens in the browser, the person signs in there with a
//! passkey and clicks Allow, and the browser comes back to a one-off address
//! here with a one-time code. This computer trades the code, with the secret
//! behind its challenge, for its own key.

use super::{Keys, Remote};
use crate::error::{Error, Result};
use crate::loopback::{Listener, Page};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::time::Duration;

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const TRADE_TIMEOUT: Duration = Duration::from_secs(20);

/// A sign-in waiting for the person in the browser.
pub struct Pending {
    /// The remote's page to open.
    pub link: String,
    listener: Listener,
    verifier: String,
    state: String,
    back: String,
}

/// Checks the remote can sign computers in, and starts listening for the
/// browser to come back.
pub async fn start(http: &reqwest::Client, remote: &Remote, computer: &str) -> Result<Pending> {
    let probe = http
        .get(format!("{}/owner/devices/connect", remote.url))
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
        .map_err(|_| {
            Error::new(
                "server.unreachable",
                format!("Couldn't reach {}.", remote.name),
            )
        })?;
    if probe.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(Error::new(
            "server.outdated",
            format!(
                "{} runs an older Hoista. Update it there, then try again.",
                remote.name
            ),
        ));
    }

    let verifier = random_text(32);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_text(16);
    let listener = Listener::bind(0, "/done").await?;
    let back = listener.address("/done");
    let query: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("name", computer)
        .append_pair("redirect", &back)
        .append_pair("challenge", &challenge)
        .append_pair("state", &state)
        .finish();
    Ok(Pending {
        link: format!("{}/owner/devices/connect?{query}", remote.url),
        listener,
        verifier,
        state,
        back,
    })
}

/// Waits for the browser to come back, trades its code for this computer's
/// key and keeps the key. A failure is a sentence for the person.
pub async fn finish(
    mut pending: Pending,
    http: &reqwest::Client,
    remote: &Remote,
    keys: &Keys,
) -> Result<(), String> {
    while let Some(visit) = pending.listener.next().await {
        if visit.get("state") != Some(pending.state.as_str()) {
            visit.answer(Page::new(
                false,
                "Couldn’t sign in",
                "This answer is for a different sign-in. Start again in the Hoista app.",
            ));
            continue;
        }
        if visit.get("error").is_some() {
            visit.answer(Page::new(
                false,
                "Couldn’t sign in",
                "You cancelled it. You can close this tab.",
            ));
            return Err("You cancelled it on its page.".to_owned());
        }
        let code = visit.get("code").unwrap_or_default().to_owned();
        let outcome = trade(http, remote, &code, &pending)
            .await
            .and_then(|key| keys.write(&remote.id, &key).map_err(|error| error.message));
        match &outcome {
            Ok(()) => visit.answer(Page::new(
                true,
                "You’re signed in",
                "Go back to the Hoista app. You can close this tab.",
            )),
            Err(message) => visit.answer(Page::new(false, "Couldn’t sign in", message)),
        }
        return outcome;
    }
    Err("The sign-in stopped. Try again.".to_owned())
}

async fn trade(
    http: &reqwest::Client,
    remote: &Remote,
    code: &str,
    pending: &Pending,
) -> Result<String, String> {
    #[derive(Deserialize)]
    struct Traded {
        key: Option<String>,
        message: Option<String>,
    }
    let unreachable = || {
        format!(
            "Couldn't reach {}. Try again in the Hoista app.",
            remote.name
        )
    };
    let response = http
        .post(format!("{}/owner/devices/token", remote.url))
        .json(&json!({ "code": code, "verifier": pending.verifier, "redirect": pending.back }))
        .timeout(TRADE_TIMEOUT)
        .send()
        .await
        .map_err(|_| unreachable())?;
    let ok = response.status().is_success();
    let traded = response.json::<Traded>().await.ok();
    match traded {
        Some(Traded { key: Some(key), .. }) if ok => Ok(key),
        Some(Traded {
            message: Some(message),
            ..
        }) => Err(message),
        _ => Err("The sign-in ended. Try again.".to_owned()),
    }
}

fn random_text(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buffer);
    URL_SAFE_NO_PAD.encode(buffer)
}
