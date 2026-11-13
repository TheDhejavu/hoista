//! Owner-approved enrollment for local (stdio) coding tools, by short code.
//!
//! Only the bridge holding the request's channel secret can collect the
//! credential, and only once. Each request makes a fresh identity; requests
//! survive a restart and expire after ten minutes.

use crate::accounts::AccountService;
use crate::engine::{Actor, Engine};
use crate::{
    domain::{
        Secret,
        model::{Agent, AgentGrantInput, AgentTransport},
    },
    ports::catalog::EngineError,
};
use sha2::{Digest, Sha256};

const MAX_NAME_BYTES: usize = 80;

/// How long a request waits for the person.
const WAIT_MINUTES: i64 = 10;
/// At most this many requests wait at once; more are refused, not queued.
const MAX_PENDING: usize = 20;
/// Characters that can't be misread for one another.
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

/// Where a request stands.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum EnrollmentStatus {
    /// Waiting for the person.
    Pending,
    /// Allowed; the bridge can collect its credential once.
    Allowed,
    /// The person said no.
    Denied,
    /// Nobody answered in time.
    Expired,
    /// Allowed and collected; nothing more to hand over.
    Collected,
}

/// A request as the person sees it. Never holds the channel secret or credential.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct EnrollmentRequest {
    /// Request id.
    pub id: String,
    /// The name the tool gave. Hoista cannot verify it.
    pub name: String,
    /// How it connects.
    pub transport: AgentTransport,
    /// Short code the bridge shows, to match the request with the tool.
    pub code: String,
    /// RFC 3339 UTC.
    pub created_at: String,
    /// RFC 3339 UTC; after this it can't be allowed.
    pub expires_at: String,
    /// Where it stands.
    pub status: EnrollmentStatus,
    /// Whether the tool can show Hoista's questions (MCP elicitation);
    /// unknown when the bridge didn't say.
    pub can_ask: Option<bool>,
}

/// What the bridge learns when it asks about its request.
#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct EnrollmentPoll {
    /// Where it stands.
    pub status: EnrollmentStatus,
    /// The new credential, given once when allowed.
    pub credential: Option<String>,
}

/// A request as Hoista keeps it, so it outlives an engine restart. Holds a
/// digest of the bridge's channel secret, never the secret or a credential;
/// once allowed, the credential is read from the keychain under `agent_id`
/// when the bridge collects it.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredEnrollment {
    /// The request as the person sees it.
    pub request: EnrollmentRequest,
    /// Lowercase hex SHA-256 of the channel secret.
    pub channel: String,
    /// The agent made when it was allowed.
    pub agent_id: Option<String>,
}

/// Keeps reads and answers about requests one at a time.
#[derive(Default)]
pub(crate) struct Enrollments(tokio::sync::Mutex<()>);

fn digest(secret: &str) -> String {
    Sha256::digest(secret.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn code() -> String {
    let bytes = uuid::Uuid::new_v4().into_bytes();
    let letters: String = bytes
        .iter()
        .take(6)
        .map(|byte| CODE_ALPHABET[usize::from(*byte) % CODE_ALPHABET.len()] as char)
        .collect();
    format!("{}-{}", &letters[..3], &letters[3..])
}

fn stamp(at: chrono::DateTime<chrono::Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

impl<'a> AccountService<'a> {
    /// Saved requests with timed-out ones marked expired. Finished requests
    /// are kept ten more minutes so the bridge can read the outcome.
    async fn current_enrollments(self) -> Result<Vec<StoredEnrollment>, EngineError> {
        let at = chrono::Utc::now();
        self.engine
            .repository
            .forget_enrollments(&stamp(at - chrono::Duration::minutes(WAIT_MINUTES)))
            .await?;
        let mut entries = self.engine.repository.enrollments().await?;
        for entry in &mut entries {
            let ends = chrono::DateTime::parse_from_rfc3339(&entry.request.expires_at)
                .map(|end| end.with_timezone(&chrono::Utc))
                .unwrap_or(at);
            if entry.request.status == EnrollmentStatus::Pending && at >= ends {
                entry.request.status = EnrollmentStatus::Expired;
                self.engine.repository.put_enrollment(entry).await?;
            }
        }
        Ok(entries)
    }
}

impl Engine {
    /// The connections still in use that go by `name` over `transport`.
    ///
    /// # Errors
    /// Reports unavailable persistent state.
    pub(crate) async fn connections_named(
        &self,
        name: &str,
        transport: AgentTransport,
    ) -> Result<Vec<Agent>, EngineError> {
        let mut found = Vec::new();
        let mut cursor = None;
        loop {
            let page = self.repository.agents(cursor.as_deref()).await?;
            found.extend(page.items.into_iter().filter(|agent| {
                !agent.revoked && agent.transport == transport && agent.name == name
            }));
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return Ok(found),
            }
        }
    }
}

impl<'a> AccountService<'a> {
    /// A coding tool asks to connect. Returns the request and the channel
    /// secret only the asking bridge receives.
    ///
    /// # Errors
    /// Rejects a paused Hoista, a bad name or too many waiting requests.
    pub async fn request_enrollment(
        self,
        name: &str,
        can_ask: Option<bool>,
    ) -> Result<(EnrollmentRequest, Secret), EngineError> {
        self.engine.require_agents_enabled().await?;
        let name = name.trim();
        if name.is_empty() || name.len() > MAX_NAME_BYTES || name.chars().any(char::is_control) {
            return Err(EngineError::Invalid(
                "Give the coding tool a name of up to 80 characters.",
            ));
        }
        let now = chrono::Utc::now();
        let _one = self.engine.enrollments.0.lock().await;
        let entries = self.engine.accounts().current_enrollments().await?;
        if entries
            .iter()
            .filter(|entry| entry.request.status == EnrollmentStatus::Pending)
            .count()
            >= MAX_PENDING
        {
            return Err(EngineError::Conflict(
                "Too many connection requests are waiting. Answer or let them expire first.",
            ));
        }
        let channel = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let request = EnrollmentRequest {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_owned(),
            transport: AgentTransport::Stdio,
            code: code(),
            created_at: stamp(now),
            expires_at: stamp(now + chrono::Duration::minutes(WAIT_MINUTES)),
            status: EnrollmentStatus::Pending,
            can_ask,
        };
        self.engine
            .repository
            .put_enrollment(&StoredEnrollment {
                request: request.clone(),
                channel: digest(&channel),
                agent_id: None,
            })
            .await?;
        drop(_one);
        self.engine
            .repository
            .event(
                "engine",
                None,
                "agent.connection_requested",
                &format!("{name} asked to connect (code {}).", request.code),
            )
            .await?;
        Ok((request, Secret::new(channel)))
    }

    /// The bridge asks about its own request with its channel secret. The
    /// credential is handed over once; a wrong secret learns nothing.
    ///
    /// # Errors
    /// Reports a missing request or a wrong channel secret as not found.
    pub async fn poll_enrollment(
        self,
        id: &str,
        channel: &str,
    ) -> Result<EnrollmentPoll, EngineError> {
        let _one = self.engine.enrollments.0.lock().await;
        let mut entry = self
            .engine
            .accounts()
            .current_enrollments()
            .await?
            .into_iter()
            .find(|entry| entry.request.id == id)
            .ok_or(EngineError::NotFound)?;
        // Compare digests, not the secret itself.
        if !bool::from(subtle::ConstantTimeEq::ct_eq(
            entry.channel.as_bytes(),
            digest(channel).as_bytes(),
        )) {
            return Err(EngineError::NotFound);
        }
        if entry.request.status != EnrollmentStatus::Allowed {
            return Ok(EnrollmentPoll {
                status: entry.request.status,
                credential: None,
            });
        }
        let agent = entry.agent_id.clone().ok_or(EngineError::Store)?;
        let credential = self.engine.vault.read(&format!("agent/{agent}")).await?;
        entry.request.status = EnrollmentStatus::Collected;
        self.engine.repository.put_enrollment(&entry).await?;
        Ok(EnrollmentPoll {
            status: EnrollmentStatus::Allowed,
            credential: Some(credential.expose().to_owned()),
        })
    }

    /// Requests waiting for the person, oldest first.
    ///
    /// # Errors
    /// Reports unavailable persistent state.
    pub async fn enrollment_requests(self) -> Result<Vec<EnrollmentRequest>, EngineError> {
        let _one = self.engine.enrollments.0.lock().await;
        let entries = self.engine.accounts().current_enrollments().await?;
        let mut waiting: Vec<EnrollmentRequest> = entries
            .iter()
            .filter(|entry| entry.request.status == EnrollmentStatus::Pending)
            .map(|entry| entry.request.clone())
            .collect();
        waiting.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(waiting)
    }

    /// The connections still in use from coding tools on this computer that
    /// go by `name`: what a new connection from the same tool replaces.
    ///
    /// # Errors
    /// Reports unavailable persistent state.
    pub async fn earlier_connections(self, name: &str) -> Result<Vec<Agent>, EngineError> {
        self.engine
            .connections_named(name, AgentTransport::Stdio)
            .await
    }

    /// The person allows a request with the access they chose, or denies it.
    /// Allowing makes a fresh identity; nothing is merged by name. With
    /// `replace_earlier`, the person's earlier connections from the same tool
    /// on this computer are revoked once the new one is allowed, so
    /// reconnecting a tool doesn't leave its old connection behind.
    ///
    /// # Errors
    /// Rejects agent callers, answered or expired requests and invalid access.
    pub async fn decide_enrollment(
        self,
        actor: &Actor,
        id: &str,
        allow: bool,
        scopes: Vec<String>,
        grant: Option<AgentGrantInput>,
        replace_earlier: bool,
    ) -> Result<EnrollmentRequest, EngineError> {
        if !matches!(actor, Actor::Person) {
            return Err(EngineError::Conflict(
                "Only the person can answer a connection request.",
            ));
        }
        let one = self.engine.enrollments.0.lock().await;
        let mut entry = self
            .engine
            .accounts()
            .current_enrollments()
            .await?
            .into_iter()
            .find(|entry| entry.request.id == id)
            .ok_or(EngineError::NotFound)?;
        if entry.request.status != EnrollmentStatus::Pending {
            return Err(EngineError::Conflict(
                "This request was already answered or has expired.",
            ));
        }
        if !allow {
            entry.request.status = EnrollmentStatus::Denied;
            self.engine.repository.put_enrollment(&entry).await?;
            drop(one);
            self.engine
                .repository
                .event(
                    actor.id(),
                    None,
                    "agent.connection_denied",
                    &format!("Didn't allow {} to connect.", entry.request.name),
                )
                .await?;
            return Ok(entry.request);
        }
        // Whoever can open the consent page or reach the owner connection
        // could be the agent itself; the owner prompt can't be.
        drop(one);
        self.engine
            .confirm_owner(&format!("let {} use your Hoista", entry.request.name))
            .await?;
        let one = self.engine.enrollments.0.lock().await;
        let Some(mut entry) = self
            .engine
            .accounts()
            .current_enrollments()
            .await?
            .into_iter()
            .find(|entry| {
                entry.request.id == id && entry.request.status == EnrollmentStatus::Pending
            })
        else {
            return Err(EngineError::Conflict(
                "This request was already answered or has expired.",
            ));
        };
        let name = entry.request.name.clone();
        let earlier = if replace_earlier {
            self.engine.accounts().earlier_connections(&name).await?
        } else {
            Vec::new()
        };
        let (agent, _token) = {
            let _mutation = self.engine.settings_changes.lock().await;
            self.engine
                .register_agent_locked(name.clone(), scopes, AgentTransport::Stdio)
                .await?
        };
        if let Some(grant) = grant {
            self.engine
                .apply_agent_grant(actor, &agent.id, grant)
                .await?;
        }
        entry.agent_id = Some(agent.id);
        entry.request.status = EnrollmentStatus::Allowed;
        self.engine.repository.put_enrollment(&entry).await?;
        drop(one);
        for old in &earlier {
            let _mutation = self.engine.settings_changes.lock().await;
            self.engine
                .repository
                .edit_agent(actor.id(), &old.id, None, true)
                .await?;
        }
        if !earlier.is_empty() {
            self.engine
                .repository
                .event(
                    actor.id(),
                    None,
                    "agent.replaced",
                    &format!("The new {name} connection replaced the earlier one."),
                )
                .await?;
        }
        Ok(entry.request)
    }
}
