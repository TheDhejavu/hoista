//! Refreshing a resource's links from its bindings at the provider. Writes nothing at the provider.

use crate::engine::Actor;
use crate::projects::ProjectService;
use crate::{domain::model::*, ports::catalog::EngineError};
use sha2::{Digest, Sha256};
use std::time::Duration;

const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// What one binding refresh covered. A binding that can't be matched to one
/// resource is left alone, never guessed.
#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct BindingRefresh {
    /// The resource whose bindings were read.
    pub resource_id: String,
    /// Bindings resolved to resources in a project, on the same account.
    pub resolved: usize,
    /// Bindings left unknown: unsupported, pointing outside a project, missing or ambiguous.
    pub unresolved: usize,
    /// When the read finished.
    pub observed_at: String,
    /// Always false: bindings aren't every possible dependency.
    pub relationships_complete: bool,
}

impl<'a> ProjectService<'a> {
    /// Reads the resource's bindings at the provider, and updates the links
    /// read from them before. Links the person drew are never touched.
    ///
    /// # Errors
    /// Fails without access, for a resource that isn't in a project or whose
    /// provider doesn't report bindings, a read that fails or takes too long,
    /// or more links than a resource can have.
    pub async fn refresh_bindings(
        self,
        actor: &Actor,
        id: &str,
    ) -> Result<BindingRefresh, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:write").await?;
        let resource = self
            .engine
            .repository
            .resource(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if resource.project_id.is_none() {
            return Err(EngineError::Invalid("Select a resource in a project."));
        }
        let adapter = self.engine.adapter(resource.provider)?;
        if !adapter.reports_bindings(&resource.provider_ref) {
            return Err(EngineError::Unsupported(format!(
                "{} doesn't report this resource's bindings.",
                resource.provider.display_name()
            )));
        }
        let account = self
            .engine
            .repository
            .account(&resource.account_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let token = self.engine.accounts().account_token(&account.id).await?;
        let bindings = tokio::time::timeout(
            READ_TIMEOUT,
            adapter.resource_bindings(&token, &account.external_id, &resource.provider_ref),
        )
        .await
        .map_err(|_| EngineError::Provider {
            evidence: None,
            code: "map.observation_timeout".into(),
            message: "Binding observation timed out; previous relationships were retained.".into(),
        })??;
        self.engine.authorize(actor, "apps:write").await?;
        let resources = self.engine.projects().all_resources().await?;
        let mut links = Vec::new();
        let mut preserve = Vec::new();
        for binding in bindings {
            let candidates: Vec<_> = resources
                .iter()
                .filter(|target| {
                    target.id != id
                        && target.account_id == resource.account_id
                        && target.project_id.is_some()
                        && binding.target_reference.as_deref() == Some(target.provider_ref.as_str())
                })
                .collect();
            if candidates.len() != 1 {
                preserve.push(binding.name);
                continue;
            }
            let target = candidates[0];
            let identity = serde_json::to_vec(&(id, &target.id, &binding.name))
                .map_err(|_| EngineError::Store)?;
            links.push(Link {
                id: format!("{OBSERVED_LINK_PREFIX}{:x}", Sha256::digest(identity)),
                from_resource: id.into(),
                to_resource: target.id.clone(),
                via: binding.name,
                derived: true,
                missing: false,
            });
        }
        self.engine
            .repository
            .reconcile_observed_links(actor.id(), id, &links, &preserve)
            .await?;
        Ok(BindingRefresh {
            resource_id: id.into(),
            resolved: links.len(),
            unresolved: preserve.len(),
            observed_at: chrono::Utc::now().to_rfc3339(),
            relationships_complete: false,
        })
    }
}
