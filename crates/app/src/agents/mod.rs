//! Agent enrollment and per-request authentication.

use crate::engine::{Actor, Engine, now};
use crate::{
    domain::{Secret, model::*},
    ports::catalog::EngineError,
};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const MAX_GRANT_PROJECTS: usize = 500;
const MAX_EXCEPTIONS: usize = 200;
const MAX_CREDENTIAL_BYTES: usize = 128;
/// Later than any enrollment started, so forgetting those before it forgets all.
const END_OF_TIME: &str = "9999-12-31T23:59:59Z";

impl Engine {
    pub(crate) async fn require_agents_enabled(&self) -> Result<(), EngineError> {
        if !self.repository.settings().await?.agents_enabled {
            return Err(EngineError::AgentsDisabled);
        }
        Ok(())
    }

    pub(crate) async fn register_agent_locked(
        &self,
        name: String,
        scopes: Vec<String>,
        transport: AgentTransport,
    ) -> Result<(Agent, Secret), EngineError> {
        self.require_agents_enabled().await?;
        if name.trim().is_empty()
            || name.len() > 80
            || name.chars().any(char::is_control)
            || !valid_agent_scopes(&scopes)
        {
            return Err(EngineError::Invalid(
                "Provide an agent name and distinct apps:read, apps:write or apps:create scopes.",
            ));
        }
        let agent = Agent {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            transport,
            scopes,
            last_seen: now(),
            revoked: false,
            grant: Default::default(),
        };
        let token = Secret::new(format!(
            "{}.{}{}",
            agent.id,
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        ));
        self.vault
            .write(
                &format!("agent/{}", agent.id),
                Secret::new(token.expose().into()),
            )
            .await?;
        if let Err(error) = self.repository.put_agent(&agent).await {
            self.vault.delete(&format!("agent/{}", agent.id)).await?;
            return Err(error);
        }
        self.repository
            .event(
                &agent.id,
                None,
                "agent.connected",
                "Registered an agent with explicit scopes.",
            )
            .await?;
        Ok((agent, token))
    }

    /// Saves a grant the person already confirmed.
    pub(crate) async fn apply_agent_grant(
        &self,
        actor: &Actor,
        id: &str,
        mut grant: crate::domain::model::AgentGrantInput,
    ) -> Result<Agent, EngineError> {
        if !matches!(actor, Actor::Person) {
            return Err(EngineError::Conflict(
                "Only the local person can change what an agent may touch.",
            ));
        }
        let tidy = |values: &mut Vec<String>| {
            for value in values.iter_mut() {
                *value = value.trim().to_owned();
            }
            values.sort();
            values.dedup();
        };
        if let Some(projects) = grant.projects.as_mut() {
            tidy(projects);
            if projects.len() > MAX_GRANT_PROJECTS {
                return Err(EngineError::Invalid("Choose at most 500 projects."));
            }
            for project in projects.iter() {
                if self.repository.project(project).await?.is_none() {
                    return Err(EngineError::Invalid(
                        "One of the chosen projects doesn't exist. Refresh and choose again.",
                    ));
                }
            }
        }
        if let Some(ids) = grant.environment_ids.as_mut() {
            tidy(ids);
            let known = self.repository.environments().await?;
            if ids.is_empty()
                || ids
                    .iter()
                    .any(|id| !known.iter().any(|environment| environment.id == *id))
            {
                return Err(EngineError::Invalid(
                    "Choose at least one environment that exists. Refresh and choose again.",
                ));
            }
        }
        if !grant.environment_rules.is_empty() {
            let known = self.repository.environments().await?;
            if grant
                .environment_rules
                .keys()
                .any(|id| !known.iter().any(|environment| environment.id == *id))
            {
                return Err(EngineError::Invalid(
                    "One of the environments doesn't exist. Refresh and choose again.",
                ));
            }
        }
        if grant.exceptions.len() > MAX_EXCEPTIONS {
            return Err(EngineError::Invalid("Keep it to 200 exceptions."));
        }
        for exception in &mut grant.exceptions {
            match (&exception.resource_id, exception.group) {
                (Some(resource), None) => {
                    if self.repository.resource(resource).await?.is_none() {
                        return Err(EngineError::Invalid(
                            "One of the exceptions names a resource that doesn't exist. Refresh and choose again.",
                        ));
                    }
                }
                (None, Some(_)) => {
                    if let Some(project) = &exception.project_id
                        && self.repository.project(project).await?.is_none()
                    {
                        return Err(EngineError::Invalid(
                            "One of the exceptions names a project that doesn't exist. Refresh and choose again.",
                        ));
                    }
                }
                _ => {
                    return Err(EngineError::Invalid(
                        "Each exception names either one resource, or a group of resources and where.",
                    ));
                }
            }
        }
        grant.exceptions.dedup();
        tidy(&mut grant.deny);
        if grant.deny.len() > 200
            || grant.deny.iter().any(|name| {
                name.is_empty()
                    || name.len() > 64
                    || !name.chars().all(|c| c.is_ascii_lowercase() || c == '_')
            })
        {
            return Err(EngineError::Invalid(
                "Name denied tools exactly, like delete_secret.",
            ));
        }
        if let Some(end) = &grant.expires_at {
            let parsed = chrono::DateTime::parse_from_rfc3339(end)
                .map_err(|_| EngineError::Invalid("Give the end date as an RFC 3339 time."))?;
            if parsed <= chrono::Utc::now() {
                return Err(EngineError::Invalid("Choose an end date in the future."));
            }
        }
        let _mutation = self.settings_changes.lock().await;
        self.repository.set_agent_grant(actor.id(), id, grant).await
    }
}

impl<'a> AgentService<'a> {
    /// Registers a local coding agent and returns its credential exactly once.
    ///
    /// # Errors
    /// Rejects invalid names, unsupported scopes or inaccessible credentials.
    pub async fn register_agent(
        self,
        name: String,
        scopes: Vec<String>,
        transport: AgentTransport,
    ) -> Result<(Agent, Secret), EngineError> {
        let _mutation = self.engine.settings_changes.lock().await;
        self.engine
            .register_agent_locked(name, scopes, transport)
            .await
    }

    /// Revokes an agent from the owner's settings. Runs under the same lock as enrollment and
    /// reconnection.
    ///
    /// # Errors
    /// Rejects non-owner callers or missing identities.
    pub async fn revoke_agent(self, actor: &Actor, id: &str) -> Result<Agent, EngineError> {
        if !matches!(actor, Actor::Person) {
            return Err(EngineError::ApprovalRequired);
        }
        let _mutation = self.engine.settings_changes.lock().await;
        self.engine
            .repository
            .edit_agent(actor.id(), id, None, true)
            .await
    }

    /// For when something looks wrong: every agent's access is revoked (each
    /// must be approved again to reconnect), agent access is paused, and
    /// waiting connection requests are dropped. Projects keep running; nothing at
    /// any provider is changed. Returns how many agents were cut off.
    ///
    /// # Errors
    /// Refuses anyone but the person; reports unavailable state.
    pub async fn stop_all_agents(self, actor: &Actor) -> Result<usize, EngineError> {
        if !matches!(actor, Actor::Person) {
            return Err(EngineError::ApprovalRequired);
        }
        let _mutation = self.engine.settings_changes.lock().await;
        // Paused first, so nothing new gets in while grants are revoked.
        let mut settings = self.engine.repository.settings().await?;
        settings.agents_enabled = false;
        self.engine.repository.put_settings(&settings).await?;
        let mut revoked = 0;
        let mut cursor = None;
        loop {
            let page = self.engine.repository.agents(cursor.as_deref()).await?;
            for agent in page.items {
                if !agent.revoked {
                    self.engine
                        .repository
                        .edit_agent(actor.id(), &agent.id, None, true)
                        .await?;
                    revoked += 1;
                }
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        self.engine
            .repository
            .forget_enrollments(END_OF_TIME)
            .await?;
        self.engine
            .repository
            .event(
                actor.id(),
                None,
                "agents.stopped",
                &format!("Cut off every agent ({revoked}) and paused agent access."),
            )
            .await?;
        Ok(revoked)
    }

    /// Reconnects a local coding tool with a new credential and the same scopes. Old
    /// credentials stay invalid; the settings UI never gets the token.
    ///
    /// # Errors
    /// Rejects non-owner callers, paused access, remote OAuth clients or vault/storage failures.
    pub async fn reconnect_agent(self, actor: &Actor, id: &str) -> Result<Agent, EngineError> {
        if !matches!(actor, Actor::Person) {
            return Err(EngineError::ApprovalRequired);
        }
        let _mutation = self.engine.settings_changes.lock().await;
        self.engine.require_agents_enabled().await?;
        let agent = self
            .engine
            .repository
            .agent(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if agent.transport != AgentTransport::Stdio {
            return Err(EngineError::Conflict(
                "Reconnect chat apps through their OAuth connection flow.",
            ));
        }
        let token = Secret::new(format!(
            "{}.{}{}",
            agent.id,
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        ));
        // Rotate first: if the SQLite transaction fails, the old credential remains
        // invalid and the owner can retry. Never revive a revoked token.
        self.engine
            .vault
            .write(&format!("agent/{}", agent.id), token)
            .await?;
        self.engine
            .repository
            .reconnect_agent(actor.id(), id, &now())
            .await
    }

    /// Changes an agent's permissions. Only the person can, from settings.
    ///
    /// # Errors
    /// Rejects agent callers, invalid scopes and revoked or missing identities.
    pub async fn set_agent_scopes(
        self,
        actor: &Actor,
        id: &str,
        scopes: Vec<String>,
    ) -> Result<Agent, EngineError> {
        if !matches!(actor, Actor::Person) {
            return Err(EngineError::Conflict(
                "Only the local person can change agent permissions.",
            ));
        }
        let current = self
            .engine
            .repository
            .agent(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if scopes.iter().any(|scope| !current.scopes.contains(scope)) {
            self.engine
                .confirm_owner(&format!("give {} more access to your Hoista", current.name))
                .await?;
        }
        let _mutation = self.engine.settings_changes.lock().await;
        self.engine
            .repository
            .edit_agent(actor.id(), id, Some(&scopes), false)
            .await
    }

    /// Changes which projects and environments an agent may touch, the tools it
    /// may never use and when its grant ends. Only the local person can.
    ///
    /// # Errors
    /// Rejects agent callers, unknown projects, malformed values and revoked identities.
    pub async fn set_agent_grant(
        self,
        actor: &Actor,
        id: &str,
        grant: crate::domain::model::AgentGrantInput,
    ) -> Result<Agent, EngineError> {
        if !matches!(actor, Actor::Person) {
            return Err(EngineError::Conflict(
                "Only the local person can change what an agent may touch.",
            ));
        }
        let current = self
            .engine
            .repository
            .agent(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if widens(&current.grant, &grant) {
            self.engine
                .confirm_owner(&format!("give {} more access to your Hoista", current.name))
                .await?;
        }
        self.engine.apply_agent_grant(actor, id, grant).await
    }

    /// Works out the actor from the agent's credential, never from a field the caller sends.
    ///
    /// # Errors
    /// Rejects malformed, invalid, revoked or expired credentials.
    pub async fn authenticate_agent(self, credential: &str) -> Result<Actor, EngineError> {
        self.engine.require_agents_enabled().await?;
        let (id, _) = credential
            .split_once('.')
            .ok_or(EngineError::ApprovalRequired)?;
        uuid::Uuid::parse_str(id).map_err(|_| EngineError::ApprovalRequired)?;
        if credential.len() > MAX_CREDENTIAL_BYTES {
            return Err(EngineError::ApprovalRequired);
        }
        // Unknown or revoked agents are refused before the keychain is read,
        // so an old credential never makes the system ask for a password.
        let agent = self
            .engine
            .repository
            .agent(id)
            .await?
            .ok_or(EngineError::ApprovalRequired)?;
        if agent.revoked {
            return Err(EngineError::ApprovalRequired);
        }
        let saved = self
            .engine
            .vault
            .read(&format!("agent/{id}"))
            .await
            .map_err(|error| match error {
                EngineError::NotFound => EngineError::ApprovalRequired,
                other => other,
            })?;
        let expected = Sha256::digest(saved.expose().as_bytes());
        let actual = Sha256::digest(credential.as_bytes());
        if !bool::from(expected.as_slice().ct_eq(actual.as_slice())) {
            return Err(EngineError::ApprovalRequired);
        }
        let actor = Actor::Agent { id: id.into() };
        let settings = self.engine.repository.settings().await?;
        let seen = chrono::DateTime::parse_from_rfc3339(&agent.last_seen)
            .map_err(|_| EngineError::Store)?;
        if agent.revoked
            || chrono::Utc::now().signed_duration_since(seen).num_days()
                >= i64::from(settings.agent_expiry_days)
        {
            return Err(EngineError::ApprovalRequired);
        }
        if agent.transport == AgentTransport::Link {
            let grant = self
                .engine
                .repository
                .oauth_grant(id)
                .await?
                .ok_or(EngineError::ApprovalRequired)?;
            let origin = self
                .engine
                .public_url()
                .ok_or(EngineError::ApprovalRequired)?;
            let expiry = chrono::DateTime::parse_from_rfc3339(&grant.expires_at)
                .map_err(|_| EngineError::Store)?;
            if grant.generation != crate::accounts::oauth::GRANT_GENERATION
                || grant.resource != format!("{origin}/mcp")
                || expiry <= chrono::Utc::now()
            {
                return Err(EngineError::ApprovalRequired);
            }
        }
        Ok(actor)
    }
}

/// Whether `next` lets the agent do anything `current` didn't.
fn widens(
    current: &crate::domain::model::AgentGrant,
    next: &crate::domain::model::AgentGrantInput,
) -> bool {
    use crate::domain::model::ProductionRule;
    fn strictness(rule: Option<ProductionRule>) -> u8 {
        match rule {
            None => 0,
            Some(ProductionRule::AlwaysAsk) => 1,
            Some(ProductionRule::ViewOnly) => 2,
        }
    }

    fn more(current: Option<&Vec<String>>, next: Option<&Vec<String>>) -> bool {
        match (current, next) {
            (None, _) => false,
            (Some(_), None) => true,
            (Some(old), Some(new)) => new.iter().any(|item| !old.contains(item)),
        }
    }
    let later = match (&current.expires_at, &next.expires_at) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(old), Some(new)) => {
            match (
                chrono::DateTime::parse_from_rfc3339(old),
                chrono::DateTime::parse_from_rfc3339(new),
            ) {
                (Ok(old), Ok(new)) => new > old,
                _ => true,
            }
        }
    };
    (next.without_asking && !current.without_asking)
        || current.environment_rules.iter().any(|(id, rule)| {
            strictness(next.environment_rules.get(id).copied()) < strictness(Some(*rule))
        })
        || more(current.projects.as_ref(), next.projects.as_ref())
        || more(
            current.environment_ids.as_ref(),
            next.environment_ids.as_ref(),
        )
        || current.deny.iter().any(|tool| !next.deny.contains(tool))
        // Every exception it had must still be there, at least as strict.
        || current.exceptions.iter().any(|old| {
            !next.exceptions.iter().any(|new| {
                new.group == old.group
                    && new.project_id == old.project_id
                    && new.resource_id == old.resource_id
                    && new.access >= old.access
            })
        })
        || later
}

/// Owner passkeys for a Hoista server.
pub mod passkey;

mod service;
pub use service::AgentService;
/// The coding tools Hoista knows how to set up.
pub mod tool;
