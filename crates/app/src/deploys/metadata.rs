//! A deploy's details: its title, which app sent it, and the request key
//! that makes retrying it safe. None of them changes what the caller may do.

use crate::deploys::DeployService;
use crate::engine::{Actor, Engine};
use crate::{
    domain::{leaks::LeakScanner, model::*},
    ports::catalog::{DeploymentAdmission, EngineError},
};
use sha2::{Digest, Sha256};

const MAX_TITLE_CHARS: usize = 160;

impl Engine {
    /// Checks the title, and works out which app sent the deploy and, for an
    /// agent, its name.
    pub(crate) async fn deploy_metadata(
        &self,
        actor: &Actor,
        options: DeployOptions,
    ) -> Result<(Option<String>, DeployChannel, Option<String>), EngineError> {
        if options
            .title
            .as_ref()
            .is_some_and(|value| value.chars().any(char::is_control))
        {
            return Err(EngineError::Invalid(
                "A deployment title cannot contain control characters.",
            ));
        }
        let title = options.title.map(|value| value.trim().to_owned());
        if let Some(value) = &title {
            if value.is_empty()
                || value.chars().count() > MAX_TITLE_CHARS
                || value.chars().any(char::is_control)
            {
                return Err(EngineError::Invalid(
                    "A deployment title must contain 1–160 characters without control characters.",
                ));
            }
            let scanner = LeakScanner::new().map_err(|_| EngineError::Store)?;
            if !scanner
                .scan("deployment title", value.as_bytes(), &[])
                .is_empty()
            {
                return Err(EngineError::Invalid(
                    "Remove credentials from the deployment title.",
                ));
            }
        }
        let (channel, name) = match actor {
            Actor::Person => {
                // The CLI always says so; anything else a person uses is the desktop app.
                let channel = options.channel.unwrap_or(DeployChannel::Desktop);
                if channel == DeployChannel::Mcp {
                    return Err(EngineError::Invalid(
                        "Agent channel metadata requires an authenticated agent.",
                    ));
                }
                (channel, None)
            }
            Actor::Agent { id } => {
                let agent = self
                    .repository
                    .agent(id)
                    .await?
                    .ok_or(EngineError::NotFound)?;
                (DeployChannel::Mcp, Some(agent.name))
            }
        };
        Ok((title, channel, name))
    }
}

impl<'a> DeployService<'a> {
    /// The deploy a request key started, without running anything again.
    /// Only the person may look up someone else's.
    ///
    /// # Errors
    /// Fails without access, for a bad key or one never used, or when state
    /// can't be read.
    pub async fn deployment_request(
        self,
        actor: &Actor,
        request_key: &str,
        namespace: Option<&str>,
    ) -> Result<DeploymentRequest, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        let namespace = namespace.unwrap_or(actor.id());
        if namespace != actor.id() && !matches!(actor, Actor::Person) {
            return Err(EngineError::Invalid(
                "Only the owner may inspect another actor's deployment request.",
            ));
        }
        self.engine
            .repository
            .deployment_request(namespace, request_key)
            .await?
            .ok_or(EngineError::NotFound)
    }
}

/// The identity a retry is matched on: the request key, and a fingerprint of
/// the request. `operation` must never carry an environment value.
pub(crate) fn admission(
    actor: &Actor,
    key: Option<&str>,
    operation: serde_json::Value,
) -> Result<Option<DeploymentAdmission>, EngineError> {
    key.map(|key| {
        let bytes = serde_json::to_vec(&operation).map_err(|_| EngineError::Store)?;
        let fingerprint = format!("{:x}", Sha256::digest(bytes));
        DeploymentAdmission::new(actor.id(), key, &fingerprint)
    })
    .transpose()
}
