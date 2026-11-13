//! Delivers events to webhooks from a durable outbox: one signed delivery per
//! event per webhook, retried with backoff.
//!
//! A send connects only to the address it checked as public, with no
//! redirects, so a hostname can't be switched to a private address mid-send.

use crate::{
    application::{
        StateStore,
        ports::{
            catalog::{CatalogRepository, EngineError},
            vault::TokenVault,
        },
        webhooks::rules,
    },
    domain::model::{DeliveryState, Webhook, WebhookDelivery},
    store::SqliteStore,
};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

/// Waits before tries 2 to 6; after the sixth, a delivery stops.
const BACKOFF: [Duration; 5] = [
    Duration::from_secs(30),
    Duration::from_secs(2 * 60),
    Duration::from_secs(10 * 60),
    Duration::from_secs(60 * 60),
    Duration::from_secs(6 * 60 * 60),
];
const TICK: Duration = Duration::from_secs(3);
/// Deliveries sent per tick; the rest wait for the next one.
const DELIVERIES_PER_TICK: u32 = 20;
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);
/// Matches the "within 10 seconds" in the error a slow receiver gets.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

/// Where Hoista may send. Only tests on this computer loosen it.
#[derive(Clone, Copy)]
pub struct Policy {
    allow_private: bool,
    allow_http: bool,
}

impl Policy {
    /// Public HTTPS addresses only.
    #[must_use]
    pub const fn strict() -> Self {
        Self {
            allow_private: false,
            allow_http: false,
        }
    }

    #[cfg(test)]
    const fn local_tests() -> Self {
        Self {
            allow_private: true,
            allow_http: true,
        }
    }
}

/// Sends webhook deliveries until `stop`.
pub struct Dispatcher {
    store: SqliteStore,
    vault: Arc<dyn TokenVault>,
    policy: Policy,
}

impl Dispatcher {
    /// A dispatcher for the webhooks in `store`, with secrets in `vault`.
    #[must_use]
    pub fn new(store: SqliteStore, vault: Arc<dyn TokenVault>) -> Self {
        Self {
            store,
            vault,
            policy: Policy::strict(),
        }
    }

    /// Runs until `stop`; problems with one webhook never stop the others.
    pub async fn run(self, stop: CancellationToken) {
        loop {
            if let Err(error) = self.tick().await {
                tracing::warn!(code = "webhooks.tick_failed", %error);
            }
            tokio::select! {
                () = stop.cancelled() => return,
                () = tokio::time::sleep(TICK) => {}
            }
        }
    }

    /// Queues new events, then sends what's due.
    ///
    /// # Errors
    /// Reports unreadable state; a failed send is recorded, not returned.
    pub async fn tick(&self) -> Result<(), EngineError> {
        let webhooks = self.store.webhooks().await?;
        for (webhook, cursor) in &webhooks {
            self.queue(webhook, *cursor).await?;
        }
        let by_id: HashMap<&str, &Webhook> = webhooks
            .iter()
            .map(|(webhook, _)| (webhook.id.as_str(), webhook))
            .collect();
        let due = self
            .store
            .due_deliveries(&now(), DELIVERIES_PER_TICK)
            .await?;
        for mut delivery in due {
            let Some(webhook) = by_id.get(delivery.webhook_id.as_str()) else {
                continue;
            };
            let outcome = self.send(webhook, &delivery).await;
            settle(&mut delivery, outcome);
            self.store.put_delivery(&delivery).await?;
        }
        Ok(())
    }

    async fn queue(&self, webhook: &Webhook, cursor: i64) -> Result<(), EngineError> {
        let events = self
            .store
            .events_after(cursor)
            .await
            .map_err(|_| EngineError::Store)?;
        let Some(last) = events.last().map(|event| event.id) else {
            return Ok(());
        };
        let deliveries: Vec<WebhookDelivery> = events
            .iter()
            .filter(|event| rules::wants(webhook, &event.kind, event.project_id.as_deref()))
            .map(|event| {
                let body = serde_json::json!({
                    "id": event.id.to_string(),
                    "type": event.kind,
                    "created_at": event.at,
                    "actor": event.actor,
                    "project_id": event.project_id,
                    "subject_id": event.subject_id,
                    "summary": event.summary,
                    "webhook_id": webhook.id,
                })
                .to_string();
                WebhookDelivery {
                    id: uuid::Uuid::new_v4().to_string(),
                    webhook_id: webhook.id.clone(),
                    event_id: event.id.to_string(),
                    event_kind: event.kind.clone(),
                    state: DeliveryState::Pending,
                    attempts: 0,
                    next_attempt_at: now(),
                    last_status: None,
                    last_error: None,
                    body,
                }
            })
            .collect();
        self.store
            .queue_deliveries(&webhook.id, &deliveries, last)
            .await
    }

    async fn send(&self, webhook: &Webhook, delivery: &WebhookDelivery) -> Result<u16, String> {
        let secret = self
            .vault
            .read(&format!("webhook/{}", webhook.id))
            .await
            .map_err(|_| "Its secret isn't in the keychain.".to_owned())?;
        let url =
            url::Url::parse(&webhook.url).map_err(|_| "Its address isn't valid.".to_owned())?;
        if url.scheme() != "https" && !(self.policy.allow_http && url.scheme() == "http") {
            return Err("Webhooks go to https:// addresses only.".into());
        }
        let host = url
            .host_str()
            .ok_or("Its address has no host name.")?
            .to_owned();
        let port = url
            .port_or_known_default()
            .ok_or("Its address has no port.")?;
        let addresses: Vec<std::net::SocketAddr> = tokio::time::timeout(
            LOOKUP_TIMEOUT,
            tokio::net::lookup_host((host.as_str(), port)),
        )
        .await
        .map_err(|_| "Looking up its address timed out.".to_owned())?
        .map_err(|_| "Its address doesn't resolve.".to_owned())?
        .collect();
        let Some(first) = addresses.first().copied() else {
            return Err("Its address doesn't resolve.".into());
        };
        if !self.policy.allow_private
            && addresses.iter().any(|address| !rules::public(address.ip()))
        {
            return Err("Its address points at a private network; Hoista won't send there.".into());
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(SEND_TIMEOUT)
            .user_agent(concat!("Hoista-Webhooks/", env!("CARGO_PKG_VERSION")))
            .resolve(&host, first)
            .build()
            .map_err(|_| "Couldn't prepare the request.".to_owned())?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let signature = sign(secret.expose(), stamp, &delivery.body);
        let response = client
            .post(url)
            .header("content-type", "application/json")
            .header("hoista-event-id", &delivery.event_id)
            .header("hoista-delivery-id", &delivery.id)
            .header("hoista-signature", format!("t={stamp},v1={signature}"))
            .body(delivery.body.clone())
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    "It didn't answer within 10 seconds.".to_owned()
                } else {
                    "Couldn't connect to it.".to_owned()
                }
            })?;
        Ok(response.status().as_u16())
    }
}

/// Signs `<t>.<body>` with HMAC-SHA256.
#[must_use]
pub fn sign(secret: &str, stamp: u64, body: &str) -> String {
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
    ring::hmac::sign(&key, format!("{stamp}.{body}").as_bytes())
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn settle(delivery: &mut WebhookDelivery, outcome: Result<u16, String>) {
    delivery.attempts += 1;
    match outcome {
        Ok(status) if (200..300).contains(&status) => {
            delivery.state = DeliveryState::Delivered;
            delivery.last_status = Some(status);
            delivery.last_error = None;
            return;
        }
        Ok(status) => {
            delivery.last_status = Some(status);
            delivery.last_error = Some(format!("It answered {status}."));
        }
        Err(why) => {
            delivery.last_status = None;
            delivery.last_error = Some(why);
        }
    }
    match BACKOFF.get(delivery.attempts as usize - 1) {
        Some(wait) => {
            delivery.next_attempt_at = (chrono::Utc::now()
                + chrono::Duration::from_std(*wait).unwrap_or_default())
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        }
        None => delivery.state = DeliveryState::Failed,
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_signature_is_hmac_sha256_over_time_and_body() {
        // Known answer: HMAC-SHA256("key", "1700000000.{}").
        let ours = sign("key", 1_700_000_000, "{}");
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, b"key");
        assert!(
            ring::hmac::verify(
                &key,
                b"1700000000.{}",
                &(0..ours.len())
                    .step_by(2)
                    .map(|at| u8::from_str_radix(&ours[at..at + 2], 16).expect("hex"))
                    .collect::<Vec<_>>()
            )
            .is_ok()
        );
        assert_ne!(ours, sign("key", 1_700_000_001, "{}"));
    }

    #[test]
    fn retries_back_off_then_stop_after_six() {
        let mut delivery = WebhookDelivery {
            id: "d".into(),
            webhook_id: "w".into(),
            event_id: "1".into(),
            event_kind: "deploy.failed".into(),
            state: DeliveryState::Pending,
            attempts: 0,
            next_attempt_at: now(),
            last_status: None,
            last_error: None,
            body: "{}".into(),
        };
        for try_number in 1..=5 {
            settle(&mut delivery, Ok(500));
            assert_eq!(delivery.state, DeliveryState::Pending, "try {try_number}");
        }
        settle(&mut delivery, Err("Couldn't connect to it.".into()));
        assert_eq!(delivery.state, DeliveryState::Failed);
        assert_eq!(delivery.attempts, 6);
        let mut ok = delivery.clone();
        ok.state = DeliveryState::Pending;
        settle(&mut ok, Ok(204));
        assert_eq!(ok.state, DeliveryState::Delivered);
    }

    /// The whole path: an event is queued once, sent signed to a receiver,
    /// and a failing receiver gets tries recorded.
    #[tokio::test]
    async fn events_are_queued_once_signed_and_retried() {
        use crate::application::engine::{Actor, Engine};
        use std::sync::Mutex;
        #[derive(Default)]
        struct Vault(Mutex<HashMap<String, String>>);
        #[async_trait::async_trait]
        impl TokenVault for Vault {
            async fn read(&self, key: &str) -> Result<crate::domain::Secret, EngineError> {
                self.0
                    .lock()
                    .map_err(|_| EngineError::Store)?
                    .get(key)
                    .map(|value| crate::domain::Secret::new(value.clone()))
                    .ok_or(EngineError::NotFound)
            }

            async fn write(
                &self,
                key: &str,
                value: crate::domain::Secret,
            ) -> Result<(), EngineError> {
                self.0
                    .lock()
                    .map_err(|_| EngineError::Store)?
                    .insert(key.into(), value.expose().into());
                Ok(())
            }

            async fn delete(&self, key: &str) -> Result<(), EngineError> {
                self.0.lock().map_err(|_| EngineError::Store)?.remove(key);
                Ok(())
            }
        }
        // A receiver that records what it got and answers in turn.
        let got: Arc<Mutex<Vec<(String, String, String)>>> = Arc::default();
        let answers: Arc<Mutex<Vec<u16>>> = Arc::new(Mutex::new(vec![500, 200]));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listen");
        let address = listener.local_addr().expect("address");
        {
            let got = got.clone();
            let answers = answers.clone();
            let project = axum::Router::new().route(
                "/hook",
                axum::routing::post(move |headers: axum::http::HeaderMap, body: String| {
                    let got = got.clone();
                    let answers = answers.clone();
                    async move {
                        let header = |name: &str| {
                            headers
                                .get(name)
                                .and_then(|value| value.to_str().ok())
                                .unwrap_or_default()
                                .to_owned()
                        };
                        got.lock().expect("got").push((
                            header("hoista-event-id"),
                            header("hoista-signature"),
                            body,
                        ));
                        let status = answers.lock().expect("answers").remove(0);
                        axum::http::StatusCode::from_u16(status).expect("status")
                    }
                }),
            );
            tokio::spawn(async move {
                let _ = axum::serve(listener, project).await;
            });
        }
        let temp = tempfile::tempdir().expect("temp");
        let paths = crate::paths::StatePaths::new(Some(temp.path().join("state"))).expect("paths");
        let store = SqliteStore::open(&paths).await.expect("store");
        let vault = Arc::new(Vault::default());
        let engine = Engine::new(
            Arc::new(store.clone()),
            vault.clone(),
            Arc::new(crate::inspection::LocalInspector),
            vec![],
            Arc::new(crate::build::LocalBuilder::new(paths.root())),
        );
        // Added through the engine, which only takes public HTTPS addresses.
        assert!(
            engine
                .webhooks()
                .add_webhook(&Actor::Person, "https://127.0.0.1/hook", vec![], None)
                .await
                .is_err()
        );
        assert!(
            engine
                .webhooks()
                .add_webhook(
                    &Actor::Agent { id: "a".into() },
                    "https://agent.example/hook",
                    vec![],
                    None
                )
                .await
                .is_err()
        );
        let added = engine
            .webhooks()
            .add_webhook(
                &Actor::Person,
                "https://agent.example/hook",
                vec!["test.*".into()],
                None,
            )
            .await
            .expect("added");
        assert!(added.secret.starts_with("whsec_"));
        // Point it at the local receiver for the test, as only a test can.
        let mut local = added.webhook.clone();
        local.url = format!("http://{address}/hook");
        let (_, cursor) = store.webhooks().await.expect("webhooks")[0].clone();
        store.put_webhook(&local, cursor).await.expect("local");
        let dispatcher = Dispatcher {
            store: store.clone(),
            vault: vault.clone(),
            policy: Policy::local_tests(),
        };

        store
            .event("person", None, "test.happened", "Something happened.")
            .await
            .expect("event");
        store
            .event("person", None, "other.thing", "Not wanted.")
            .await
            .expect("event");
        dispatcher.tick().await.expect("tick");
        dispatcher.tick().await.expect("again, nothing new");
        // The receiver answered 500: one try, recorded, waiting to retry.
        let deliveries = engine
            .webhooks()
            .webhook_deliveries(&local.id)
            .await
            .expect("deliveries");
        assert_eq!(deliveries.len(), 1, "only the wanted event, queued once");
        assert_eq!(deliveries[0].event_kind, "test.happened");
        assert_eq!(deliveries[0].attempts, 1);
        assert_eq!(deliveries[0].last_status, Some(500));
        assert_eq!(deliveries[0].state, DeliveryState::Pending);
        assert!(deliveries[0].body.is_empty(), "the body isn't shown back");

        // Due again now: the same event, the same id and body, signed afresh.
        let mut due = store.deliveries(&local.id, 1).await.expect("due")[0].clone();
        due.next_attempt_at = now();
        store.put_delivery(&due).await.expect("due");
        dispatcher.tick().await.expect("retry");
        let received = got.lock().expect("got").clone();
        assert_eq!(received.len(), 2);
        assert_eq!(
            received[0].0, received[1].0,
            "the same event id on every try"
        );
        assert_eq!(received[0].2, received[1].2, "the same body on every try");
        let body: serde_json::Value = serde_json::from_str(&received[1].2).expect("json");
        assert_eq!(body["type"], "test.happened");
        assert_eq!(body["summary"], "Something happened.");
        let (stamp, signature) = received[1]
            .1
            .strip_prefix("t=")
            .and_then(|rest| rest.split_once(",v1="))
            .expect("signature header");
        assert_eq!(
            signature,
            sign(&added.secret, stamp.parse().expect("t"), &received[1].2)
        );
        let deliveries = engine
            .webhooks()
            .webhook_deliveries(&local.id)
            .await
            .expect("deliveries");
        assert_eq!(deliveries[0].state, DeliveryState::Delivered);

        // A new secret: the old one no longer matches.
        let rotated = engine
            .webhooks()
            .rotate_webhook(&Actor::Person, &local.id)
            .await
            .expect("rotated");
        assert_ne!(rotated.secret, added.secret);
        engine
            .webhooks()
            .remove_webhook(&Actor::Person, &local.id)
            .await
            .expect("removed");
        assert!(engine.webhooks().list().await.expect("webhooks").is_empty());
        assert!(vault.read(&format!("webhook/{}", local.id)).await.is_err());
        store.close().await;
    }

    #[tokio::test]
    async fn private_addresses_are_never_sent_to() {
        let temp = tempfile::tempdir().expect("temp");
        let paths = crate::paths::StatePaths::new(Some(temp.path().join("state"))).expect("paths");
        let store = SqliteStore::open(&paths).await.expect("store");
        let dispatcher = Dispatcher::new(store.clone(), Arc::new(NoSecrets));
        let webhook = Webhook {
            id: "w".into(),
            url: "https://localhost/hook".into(),
            events: vec![],
            project_id: None,
            created_at: now(),
        };
        let delivery = WebhookDelivery {
            id: "d".into(),
            webhook_id: "w".into(),
            event_id: "1".into(),
            event_kind: "x".into(),
            state: DeliveryState::Pending,
            attempts: 0,
            next_attempt_at: now(),
            last_status: None,
            last_error: None,
            body: "{}".into(),
        };
        let refused = dispatcher.send(&webhook, &delivery).await;
        assert!(
            matches!(&refused, Err(why) if why.contains("private network")),
            "{refused:?}"
        );
        store.close().await;
    }

    struct NoSecrets;
    #[async_trait::async_trait]
    impl TokenVault for NoSecrets {
        async fn read(&self, _: &str) -> Result<crate::domain::Secret, EngineError> {
            Ok(crate::domain::Secret::new("whsec_test".into()))
        }

        async fn write(&self, _: &str, _: crate::domain::Secret) -> Result<(), EngineError> {
            Ok(())
        }

        async fn delete(&self, _: &str) -> Result<(), EngineError> {
            Ok(())
        }
    }
}
