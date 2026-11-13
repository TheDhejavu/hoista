//! OAuth for remote agents: PKCE, approved by the person, revocable per client and bound
//! to one MCP URL.

use crate::accounts::AccountService;
use crate::engine::{Actor, now};
use crate::{
    domain::{Secret, model::*, oauth::*},
    ports::catalog::EngineError,
};
use base64::Engine as _;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// A PKCE challenge is a SHA-256.
const SHA256_BYTES: usize = 32;
/// The lengths PKCE allows a verifier.
const PKCE_VERIFIER_CHARS: std::ops::RangeInclusive<usize> = 43..=128;
const CODE_LIFETIME_MINUTES: i64 = 15;
/// How long an agent's OAuth access token works.
pub const ACCESS_TOKEN_SECONDS: i64 = 60 * 60;
const CODE_RANDOM_BYTES: usize = 32;

/// Random bytes for an authorization code, in hex.
fn random_hex() -> Result<String, EngineError> {
    let mut bytes = zeroize::Zeroizing::new([0u8; CODE_RANDOM_BYTES]);
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), bytes.as_mut())
        .map_err(|_| EngineError::Store)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// A remote agent waiting to connect, as the consent page shows it. Holds
/// no code, state or PKCE material.
#[derive(Clone, Debug)]
pub struct OAuthRequest {
    /// Pending request id.
    pub id: String,
    /// The name the client registered. Hoista cannot verify it.
    pub name: String,
    /// Short code shown on both pages, to match them.
    pub code: String,
    /// The provider the browser goes back to after the answer.
    pub returns_to: String,
    /// RFC 3339 deadline.
    pub expires_at: String,
}

/// The short code both pages show for one request, `ABCD-EFGH`.
#[must_use]
pub fn pairing_code(id: &str) -> String {
    const LETTERS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let digest = Sha256::digest(format!("hoista-connect\0{id}").as_bytes());
    let mut code = String::with_capacity(9);
    for (index, byte) in digest.iter().take(8).enumerate() {
        if index == 4 {
            code.push('-');
        }
        code.push(char::from(LETTERS[usize::from(*byte) % LETTERS.len()]));
    }
    code
}

/// Validated OAuth authorization-code request.
pub struct AuthorizationRequest {
    /// Registered client id.
    pub client_id: String,
    /// Exact registered callback.
    pub redirect_uri: String,
    /// The caller's own `state`, returned to it unchanged. Not empty.
    pub state: String,
    /// Base64url SHA-256 PKCE challenge.
    pub challenge: String,
    /// Exact MCP URL requested as audience.
    pub resource: String,
    /// The scopes asked for.
    pub scopes: Vec<String>,
}

/// The generation every grant records; a grant with another one is refused.
pub(crate) const GRANT_GENERATION: &str = "cloud";

impl<'a> AccountService<'a> {
    /// Registers an OAuth public client without granting it access.
    ///
    /// # Errors
    /// Rejects invalid names or unsafe callback URLs.
    pub async fn oauth_register(
        self,
        name: String,
        redirect_uris: Vec<String>,
    ) -> Result<OAuthClient, EngineError> {
        let _settings = self.engine.settings_changes.lock().await;
        self.engine.require_agents_enabled().await?;
        if name.is_empty()
            || name.len() > 80
            || name.chars().any(char::is_control)
            || redirect_uris.is_empty()
            || redirect_uris.len() > 8
        {
            return Err(EngineError::Invalid(
                "Provide a client name and up to eight redirect URIs.",
            ));
        }
        for uri in &redirect_uris {
            validate_redirect(uri)?;
        }
        let client = OAuthClient {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            redirect_uris,
        };
        self.engine.repository.put_oauth_client(&client).await?;
        Ok(client)
    }

    /// The registered client and the exact permissions an OAuth approval asks for.
    /// Callers that show this must not expose the code, state or PKCE material.
    pub async fn connection_review(
        self,
        approval: &Approval,
    ) -> Result<(OAuthPending, OAuthClient, Agent), EngineError> {
        self.engine.require_agents_enabled().await?;
        let pending = self
            .engine
            .repository
            .oauth_pending(&approval.target)
            .await?
            .ok_or(EngineError::NotFound)?;
        let client = self
            .engine
            .repository
            .oauth_client(&pending.client_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let agent = self
            .engine
            .repository
            .agent(&pending.agent_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let origin = self.engine.public_url().ok_or(EngineError::NotFound)?;
        let expires = chrono::DateTime::parse_from_rfc3339(&pending.expires_at)
            .map_err(|_| EngineError::ApprovalRequired)?;
        if approval.action != GatedAction::ConnectAgent
            || pending.id != approval.target
            || pending.approval_id != approval.id
            || pending.agent_id != approval.agent_id
            || agent.transport != AgentTransport::Link
            || agent.revoked
            || pending.requested_scopes != agent.scopes
            || !valid_agent_scopes(&agent.scopes)
            || !client.redirect_uris.contains(&pending.redirect_uri)
            || pending.resource != format!("{origin}/mcp")
            || expires <= chrono::Utc::now()
        {
            return Err(EngineError::ApprovalRequired);
        }
        Ok((pending, client, agent))
    }

    /// Starts a fifteen-minute approval and returns an authorization code held only by the browser.
    ///
    /// # Errors
    /// Rejects invalid redirects, scopes, audience and PKCE challenges.
    pub async fn oauth_authorize(
        self,
        input: AuthorizationRequest,
    ) -> Result<(OAuthPending, Secret), EngineError> {
        let _settings = self.engine.settings_changes.lock().await;
        self.engine.require_agents_enabled().await?;
        let origin = self.engine.public_url().ok_or(EngineError::NotFound)?;
        let client = self
            .engine
            .repository
            .oauth_client(&input.client_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if !client.redirect_uris.contains(&input.redirect_uri)
            || input.resource != format!("{origin}/mcp")
            || input.state.is_empty()
            || input.state.len() > 1024
            || !base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(&input.challenge)
                .is_ok_and(|value| value.len() == SHA256_BYTES)
        {
            return Err(EngineError::Invalid(
                "Invalid OAuth redirect, audience, state or S256 challenge.",
            ));
        }
        if !valid_agent_scopes(&input.scopes) {
            return Err(EngineError::Invalid(
                "Request distinct apps:read, apps:write or apps:create scopes.",
            ));
        }
        // Unapproved remote callers must not allocate credentials in the native vault.
        let agent = Agent {
            id: uuid::Uuid::new_v4().to_string(),
            name: client.name.clone(),
            transport: AgentTransport::Link,
            scopes: input.scopes,
            last_seen: now(),
            revoked: false,
            grant: Default::default(),
        };
        let id = uuid::Uuid::new_v4().to_string();
        let code = Secret::new(format!("{id}.{}", random_hex()?));
        let expiry = (chrono::Utc::now() + chrono::Duration::minutes(CODE_LIFETIME_MINUTES))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let approval = Approval {
            deletion: None,
            id: uuid::Uuid::new_v4().to_string(),
            agent_id: agent.id.clone(),
            action: GatedAction::ConnectAgent,
            target: id.clone(),
            reason: format!(
                "Allow {} to connect with {}? Callback: {}",
                client.name,
                agent.scopes.join(", "),
                input.redirect_uri
            ),
            created_at: now(),
            expires_at: expiry.clone(),
            decision: ApprovalDecision::Pending,
        };
        let pending = OAuthPending {
            requested_scopes: agent.scopes.clone(),
            id,
            client_id: client.id,
            agent_id: agent.id.clone(),
            redirect_uri: input.redirect_uri,
            state: input.state,
            challenge: input.challenge,
            resource: input.resource,
            code_hash: digest(code.expose()),
            approval_id: approval.id.clone(),
            expires_at: expiry,
        };
        self.engine
            .repository
            .begin_oauth(&agent, &approval, &pending)
            .await?;
        Ok((pending, code))
    }

    /// A remote agent waiting for the person on the consent page, or `None`
    /// once it was answered or has expired.
    ///
    /// # Errors
    /// Reports unavailable persistent state.
    pub async fn oauth_request(self, id: &str) -> Result<Option<OAuthRequest>, EngineError> {
        let Some(pending) = self.engine.repository.oauth_pending(id).await? else {
            return Ok(None);
        };
        let Some(approval) = self
            .engine
            .repository
            .approval(&pending.approval_id)
            .await?
        else {
            return Ok(None);
        };
        if approval.decision != ApprovalDecision::Pending {
            return Ok(None);
        }
        let Ok((pending, client, _)) = self.engine.accounts().connection_review(&approval).await
        else {
            return Ok(None);
        };
        Ok(Some(OAuthRequest {
            code: pairing_code(&pending.id),
            returns_to: url::Url::parse(&pending.redirect_uri)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
                .unwrap_or_default(),
            id: pending.id,
            name: client.name,
            expires_at: pending.expires_at,
        }))
    }

    /// The person's answer to a remote agent from the consent page: the
    /// access they picked replaces what the agent asked for.
    ///
    /// # Errors
    /// Rejects agent callers, invalid scopes and requests that ended.
    pub async fn decide_oauth(
        self,
        actor: &Actor,
        id: &str,
        allow: bool,
        mut scopes: Vec<String>,
        grant: AgentGrantInput,
    ) -> Result<OAuthRequest, EngineError> {
        // Stored sorted; the token step compares the two exactly.
        scopes.sort();
        if !matches!(actor, Actor::Person) {
            return Err(EngineError::Conflict(
                "Only the person can connect an agent.",
            ));
        }
        let request = self
            .engine
            .accounts()
            .oauth_request(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let mut pending = self
            .engine
            .repository
            .oauth_pending(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if !allow {
            self.engine
                .record_decision(&pending.approval_id, false, None)
                .await?;
            return Ok(request);
        }
        self.engine
            .confirm_owner(&format!("let {} use your Hoista", request.name))
            .await?;
        if !valid_agent_scopes(&scopes) {
            return Err(EngineError::Invalid(
                "Request distinct apps:read, apps:write or apps:create scopes.",
            ));
        }
        self.engine
            .repository
            .edit_agent(actor.id(), &pending.agent_id, Some(&scopes), false)
            .await?;
        pending.requested_scopes = scopes;
        self.engine.repository.put_oauth_pending(&pending).await?;
        self.engine
            .apply_agent_grant(actor, &pending.agent_id, grant)
            .await?;
        self.engine
            .record_decision(&pending.approval_id, true, None)
            .await?;
        // Connecting the same app again replaces its earlier connection, so
        // signing in once more doesn't leave the old one behind.
        let earlier: Vec<_> = self
            .engine
            .connections_named(&request.name, AgentTransport::Link)
            .await?
            .into_iter()
            .filter(|agent| agent.id != pending.agent_id)
            .collect();
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
                    &format!(
                        "The new {} connection replaced the earlier one.",
                        request.name
                    ),
                )
                .await?;
        }
        Ok(request)
    }

    /// Reads a browser's own pending authorization using its unguessable code.
    ///
    /// # Errors
    /// Rejects stale or unknown codes and does not reveal another client's metadata.
    pub async fn oauth_pending(
        self,
        code: &str,
    ) -> Result<(OAuthPending, ApprovalDecision), EngineError> {
        let (id, _) = code.split_once('.').ok_or(EngineError::ApprovalRequired)?;
        let pending = self
            .engine
            .repository
            .oauth_pending(id)
            .await?
            .ok_or(EngineError::ApprovalRequired)?;
        let actual = digest(code);
        if !bool::from(actual.as_bytes().ct_eq(pending.code_hash.as_bytes()))
            || chrono::DateTime::parse_from_rfc3339(&pending.expires_at)
                .map_err(|_| EngineError::Store)?
                <= chrono::Utc::now()
        {
            return Err(EngineError::ApprovalRequired);
        }
        let approval = self
            .engine
            .repository
            .approval(&pending.approval_id)
            .await?
            .ok_or(EngineError::ApprovalRequired)?;
        Ok((pending, approval.decision))
    }

    /// Exchanges one approved code with its exact PKCE verifier and registered callback.
    ///
    /// # Errors
    /// Rejects mismatches, expiry, revocation and reused codes.
    pub async fn oauth_token(
        self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
        verifier: &str,
        resource: &str,
    ) -> Result<Secret, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        let _settings = self.engine.settings_changes.lock().await;
        self.engine.require_agents_enabled().await?;
        let (pending, decision) = self.engine.accounts().oauth_pending(code).await?;
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        if decision != ApprovalDecision::Allowed
            || pending.client_id != client_id
            || pending.redirect_uri != redirect_uri
            || pending.resource != resource
            || !PKCE_VERIFIER_CHARS.contains(&verifier.len())
            || !verifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
            || !bool::from(challenge.as_bytes().ct_eq(pending.challenge.as_bytes()))
        {
            return Err(EngineError::ApprovalRequired);
        }
        let origin = self
            .engine
            .public_url()
            .ok_or(EngineError::ApprovalRequired)?;
        if resource != format!("{origin}/mcp") {
            return Err(EngineError::ApprovalRequired);
        }
        let agent = self
            .engine
            .repository
            .agent(&pending.agent_id)
            .await?
            .ok_or(EngineError::ApprovalRequired)?;
        if agent.revoked || pending.requested_scopes != agent.scopes {
            return Err(EngineError::ApprovalRequired);
        }
        self.engine
            .repository
            .consume_approval(
                &pending.approval_id,
                &agent.id,
                GatedAction::ConnectAgent,
                &pending.id,
                &now(),
            )
            .await?;
        let credential = Secret::new(format!(
            "{}.{}{}",
            agent.id,
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        ));
        self.engine
            .vault
            .write(
                &format!("agent/{}", agent.id),
                Secret::new(credential.expose().into()),
            )
            .await?;
        if let Err(error) = self
            .engine
            .repository
            .put_oauth_grant(&OAuthGrant {
                agent_id: agent.id.clone(),
                client_id: client_id.into(),
                resource: resource.into(),
                generation: GRANT_GENERATION.into(),
                expires_at: (chrono::Utc::now() + chrono::Duration::seconds(ACCESS_TOKEN_SECONDS))
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            })
            .await
        {
            self.engine
                .vault
                .delete(&format!("agent/{}", agent.id))
                .await?;
            return Err(error);
        }
        Ok(credential)
    }
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn validate_redirect(value: &str) -> Result<(), EngineError> {
    let uri = url::Url::parse(value)
        .map_err(|_| EngineError::Invalid("Use an absolute OAuth callback URI."))?;
    let local = matches!(uri.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    if value.len() > 2048
        || !uri.username().is_empty()
        || uri.password().is_some()
        || uri.fragment().is_some()
        || !(uri.scheme() == "https" || uri.scheme() == "http" && local)
    {
        return Err(EngineError::Invalid(
            "OAuth callbacks require HTTPS, except loopback HTTP clients.",
        ));
    }
    Ok(())
}
