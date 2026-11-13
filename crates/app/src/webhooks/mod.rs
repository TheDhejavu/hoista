//! Webhooks for background agents: the person adds one with a public
//! HTTPS address and the events it wants; Hoista signs every delivery with a
//! secret shown once and kept in the keychain. The sending is done by the
//! webhook dispatcher in `hoista-store`.

pub mod rules;

use crate::engine::{Actor, now};
use crate::{
    domain::{
        Secret,
        model::{Webhook, WebhookDelivery},
    },
    ports::catalog::EngineError,
};
use chacha20poly1305::aead::{OsRng, rand_core::RngCore};

const MAX_WEBHOOKS: usize = 20;
const MAX_URL_BYTES: usize = 2048;
const SECRET_BYTES: usize = 32;
/// Deliveries shown for one webhook, newest first.
const DELIVERIES_SHOWN: u32 = 50;

/// A new webhook, with its signing secret: shown once, never again.
#[derive(Debug, serde::Serialize, schemars::JsonSchema, ts_rs::TS)]
pub struct NewWebhook {
    /// The webhook.
    pub webhook: Webhook,
    /// Checks each delivery's `Hoista-Signature`. Keep it with the receiver.
    pub secret: String,
}

fn person(actor: &Actor) -> Result<(), EngineError> {
    if matches!(actor, Actor::Person) {
        Ok(())
    } else {
        Err(EngineError::Conflict(
            "Only the person can add, change or remove webhooks.",
        ))
    }
}

/// A new signing secret: `whsec_` and 32 random bytes in hex.
fn new_secret() -> Secret {
    let mut bytes = zeroize::Zeroizing::new([0u8; SECRET_BYTES]);
    OsRng.fill_bytes(bytes.as_mut());
    Secret::new(format!(
        "whsec_{}",
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

/// A public `https://` address with no credentials in it. Whether its name
/// leads somewhere private is checked again on every send.
fn checked_url(url: &str) -> Result<String, EngineError> {
    let parsed = url::Url::parse(url.trim())
        .map_err(|_| EngineError::Invalid("That isn't a web address."))?;
    if parsed.scheme() != "https" {
        return Err(EngineError::Invalid(
            "Webhooks go to https:// addresses only.",
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(EngineError::Invalid(
            "Leave sign-in details out of the address; check the signature instead.",
        ));
    }
    match parsed.host() {
        Some(url::Host::Domain(name)) if name.contains('.') && !name.ends_with(".local") => {}
        Some(url::Host::Ipv4(ip)) if rules::public_v4(ip) => {}
        Some(url::Host::Ipv6(ip)) if rules::public_v6(ip) => {}
        _ => {
            return Err(EngineError::Invalid(
                "Webhooks go to public addresses only, not this computer or a private network.",
            ));
        }
    }
    if parsed.as_str().len() > MAX_URL_BYTES {
        return Err(EngineError::Invalid("That address is too long."));
    }
    Ok(parsed.to_string())
}

fn checked_events(events: Vec<String>) -> Result<Vec<String>, EngineError> {
    let mut events: Vec<String> = events
        .into_iter()
        .map(|kind| kind.trim().to_owned())
        .collect();
    events.sort();
    events.dedup();
    let fine = events.len() <= 50
        && events.iter().all(|kind| {
            !kind.is_empty()
                && kind.len() <= 64
                && kind
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._*".contains(&b))
                && (!kind.contains('*') || (kind.ends_with(".*") && kind.matches('*').count() == 1))
        });
    if fine {
        Ok(events)
    } else {
        Err(EngineError::Invalid(
            "Name events like deploy.succeeded, or a family like deploy.*.",
        ))
    }
}

impl<'a> WebhookService<'a> {
    /// Adds a webhook. It gets events from now on, not earlier ones.
    ///
    /// # Errors
    /// Refuses agents, private or non-HTTPS addresses, unknown projects and bad event names.
    pub async fn add_webhook(
        self,
        actor: &Actor,
        url: &str,
        events: Vec<String>,
        project_id: Option<String>,
    ) -> Result<NewWebhook, EngineError> {
        person(actor)?;
        let url = checked_url(url)?;
        let events = checked_events(events)?;
        if let Some(project) = &project_id {
            self.engine
                .repository
                .project(project)
                .await?
                .ok_or(EngineError::NotFound)?;
        }
        if self.engine.repository.webhooks().await?.len() >= MAX_WEBHOOKS {
            return Err(EngineError::Conflict(
                "There are 20 webhooks already. Remove one first.",
            ));
        }
        let webhook = Webhook {
            id: uuid::Uuid::new_v4().to_string(),
            url,
            events,
            project_id,
            created_at: now(),
        };
        let secret = new_secret();
        self.engine
            .vault
            .write(
                &format!("webhook/{}", webhook.id),
                Secret::new(secret.expose().to_owned()),
            )
            .await?;
        let cursor = self.engine.repository.latest_event_id().await?;
        if let Err(error) = self.engine.repository.put_webhook(&webhook, cursor).await {
            let _ = self
                .engine
                .vault
                .delete(&format!("webhook/{}", webhook.id))
                .await;
            return Err(error);
        }
        self.engine
            .repository
            .event(
                actor.id(),
                webhook.project_id.as_deref(),
                "webhook.added",
                &format!("Added a webhook to {}.", host_of(&webhook.url)),
            )
            .await?;
        Ok(NewWebhook {
            webhook,
            secret: secret.expose().to_owned(),
        })
    }

    /// Every webhook.
    ///
    /// # Errors
    /// Returns unavailable persistent state.
    pub async fn list(self) -> Result<Vec<Webhook>, EngineError> {
        Ok(self
            .engine
            .repository
            .webhooks()
            .await?
            .into_iter()
            .map(|(webhook, _)| webhook)
            .collect())
    }

    /// A webhook's newest 50 deliveries.
    ///
    /// # Errors
    /// Returns unknown webhooks and unavailable persistent state.
    pub async fn webhook_deliveries(self, id: &str) -> Result<Vec<WebhookDelivery>, EngineError> {
        self.engine.webhooks().webhook(id).await?;
        let mut items = self
            .engine
            .repository
            .deliveries(id, DELIVERIES_SHOWN)
            .await?;
        for item in &mut items {
            item.body.clear();
        }
        Ok(items)
    }

    /// A new signing secret; the old one stops working straight away.
    ///
    /// # Errors
    /// Refuses agents and unknown webhooks.
    pub async fn rotate_webhook(self, actor: &Actor, id: &str) -> Result<NewWebhook, EngineError> {
        person(actor)?;
        let webhook = self.engine.webhooks().webhook(id).await?;
        let secret = new_secret();
        self.engine
            .vault
            .write(
                &format!("webhook/{id}"),
                Secret::new(secret.expose().to_owned()),
            )
            .await?;
        self.engine
            .repository
            .event(
                actor.id(),
                webhook.project_id.as_deref(),
                "webhook.rotated",
                &format!(
                    "Made a new secret for the webhook to {}.",
                    host_of(&webhook.url)
                ),
            )
            .await?;
        Ok(NewWebhook {
            webhook,
            secret: secret.expose().to_owned(),
        })
    }

    /// Removes a webhook, its secret and its delivery history.
    ///
    /// # Errors
    /// Refuses agents and unknown webhooks.
    pub async fn remove_webhook(self, actor: &Actor, id: &str) -> Result<(), EngineError> {
        person(actor)?;
        let webhook = self.engine.webhooks().webhook(id).await?;
        self.engine.repository.delete_webhook(id).await?;
        self.engine.vault.delete(&format!("webhook/{id}")).await?;
        self.engine
            .repository
            .event(
                actor.id(),
                webhook.project_id.as_deref(),
                "webhook.removed",
                &format!("Removed the webhook to {}.", host_of(&webhook.url)),
            )
            .await?;
        Ok(())
    }

    async fn webhook(self, id: &str) -> Result<Webhook, EngineError> {
        self.engine
            .repository
            .webhooks()
            .await?
            .into_iter()
            .map(|(webhook, _)| webhook)
            .find(|webhook| webhook.id == id)
            .ok_or(EngineError::NotFound)
    }
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Which events a webhook gets, and where Hoista will send them.
mod service;
pub use service::WebhookService;
