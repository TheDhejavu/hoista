//! Ignoring found resources, and undoing it. Local only; nothing changes at the provider.

use crate::accounts::AccountService;
use crate::engine::{Actor, Engine};
use crate::{
    domain::model::Resource,
    ports::catalog::{EngineError, Page},
};
use std::collections::HashSet;

impl Engine {
    /// Every resource Hoista knows about, apart from the ignored ones.
    pub(crate) async fn visible_import_resources(&self) -> Result<Vec<Resource>, EngineError> {
        let ignored: HashSet<_> = self
            .repository
            .ignored_resources()
            .await?
            .into_iter()
            .collect();
        Ok(self
            .projects()
            .all_resources()
            .await?
            .into_iter()
            .filter(|resource| !ignored.contains(&resource.id))
            .collect())
    }
}

impl<'a> AccountService<'a> {
    /// Found resources that aren't in a project: the ignored ones, or the rest.
    ///
    /// # Errors
    /// Fails without access, or when state can't be read.
    pub async fn import_candidates(
        self,
        actor: &Actor,
        ignored: bool,
        cursor: Option<&str>,
    ) -> Result<Page<Resource>, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        if cursor.is_none() {
            self.engine.look_at_new_accounts().await?;
        }
        self.engine
            .repository
            .import_candidates(ignored, cursor)
            .await
    }

    /// Ignores a resource that isn't in a project, or stops ignoring it.
    /// Nothing changes at the provider.
    ///
    /// # Errors
    /// Fails without access, for a resource that's missing or in a project,
    /// or when state can't be written.
    pub async fn set_resource_ignored(
        self,
        actor: &Actor,
        id: &str,
        ignored: bool,
    ) -> Result<(), EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:write").await?;
        self.engine
            .repository
            .set_resource_ignored(actor.id(), id, ignored)
            .await
    }
}
