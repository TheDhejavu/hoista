//! Building projects. Project code never gets provider credentials.

use crate::domain::{Secret, model::Plan};
use crate::ports::{catalog::EngineError, providers::Artifact};
use crate::{deploys::planning::Detection, domain::model::Source, engine::SourceGit};

/// Told about a build's progress as it happens.
#[async_trait::async_trait]
pub trait BuildProgress: Send {
    /// The source was copied and checked, before any of the project's own
    /// code has run. An error here stops the build.
    ///
    /// # Errors
    /// Fails when it can't be recorded.
    async fn source_snapshot(
        &mut self,
        provenance: &crate::domain::provenance::SourceProvenance,
    ) -> Result<(), EngineError>;
}

/// Builds projects for the deploy worker: copies the source, runs the build,
/// and checks everything that would be published for secrets.
#[async_trait::async_trait]
pub trait BuildService: Send + Sync {
    /// How project build code is kept apart from the rest of this computer: its
    /// name, or why it can't be. Builders that run no project code need none.
    fn isolation(&self) -> Result<&'static str, &'static str> {
        Ok("not needed")
    }

    /// The project's environment values from its `.env` files, in memory
    /// that's wiped after use. They're never saved.
    ///
    /// # Errors
    /// Fails for an unreadable `.env` file, or a secret in a variable the
    /// browser can see.
    async fn secrets(&self, plan: &Plan) -> Result<Vec<(String, Secret)>, EngineError>;

    /// Like [`Self::secrets`], with `overrides` replacing values of the same name.
    ///
    /// # Errors
    /// Reports invalid inputs, including an override that puts a secret in a
    /// browser-visible variable.
    async fn secrets_with_overrides(
        &self,
        plan: &Plan,
        overrides: Vec<(String, Secret)>,
    ) -> Result<Vec<(String, Secret)>, EngineError> {
        let mut values = self.secrets(plan).await?;
        let scanner = crate::domain::leaks::LeakScanner::new().map_err(|_| EngineError::Store)?;
        for (name, value) in overrides {
            if !scanner.public_env(&name, &value).is_empty() {
                return Err(EngineError::Invalid(
                    "A browser-visible environment variable cannot contain a secret.",
                ));
            }
            values.retain(|(existing, _)| existing != &name);
            values.push((name, value));
        }
        Ok(values)
    }

    /// Builds the project in its own workspace, refusing output that carries
    /// a secret.
    ///
    /// # Errors
    /// Fails when the source can't be copied, the build fails, the output is
    /// too big, or a secret turns up.
    async fn build(
        &self,
        plan: &Plan,
        deploy_id: &str,
        secrets: Vec<(String, Secret)>,
    ) -> Result<Artifact, EngineError>;

    /// [`Self::build`], telling `progress` once the source is copied. A real
    /// builder tells it before running any build command; this default can
    /// only tell it afterwards.
    ///
    /// # Errors
    /// As [`Self::build`], or when `progress` fails.
    async fn build_with_progress(
        &self,
        plan: &Plan,
        deploy_id: &str,
        secrets: Vec<(String, Secret)>,
        progress: &mut dyn BuildProgress,
    ) -> Result<Artifact, EngineError> {
        let artifact = self.build(plan, deploy_id, secrets).await?;
        if let Some(provenance) = &artifact.provenance {
            progress.source_snapshot(provenance).await?;
        }
        Ok(artifact)
    }

    /// Applies the project's checked-in migrations to `database`, never
    /// generating or resetting a schema. Must give up after a while, and must
    /// never save or log the connection.
    ///
    /// # Errors
    /// Fails when the migration tool is missing or a migration fails; the
    /// new release must then not go out.
    async fn migrate(
        &self,
        _plan: &Plan,
        _deploy_id: &str,
        _database: &Secret,
    ) -> Result<(), EngineError> {
        Err(EngineError::Invalid(
            "This builder does not support the planned database migration.",
        ))
    }

    /// Removes a build's workspace once the deploy is done.
    ///
    /// # Errors
    /// Fails when it can't be removed. The deploy itself stands.
    async fn cleanup(&self, deploy_id: &str) -> Result<(), EngineError>;
}

/// Reads a project's source. Reads files only; never runs anything.
#[async_trait::async_trait]
pub trait SourceInspector: Send + Sync {
    /// What can be told about the source at a glance, such as its commit. Not
    /// a checked snapshot. The deploy worker gives up on it after a while, and
    /// its failure never hides the deploy's own error.
    ///
    /// # Errors
    /// Fails when the source can't be read; the deploy then records that.
    async fn observe_source(
        &self,
        _: &Source,
    ) -> Result<crate::domain::provenance::SourceObservation, EngineError> {
        Ok(crate::domain::provenance::SourceObservation::Unavailable)
    }

    /// How to deploy the source: its framework, build and variable names.
    /// Never the variables' values.
    ///
    /// # Errors
    /// Fails for a source that can't be read or that Hoista can't deploy.
    async fn detect(&self, source: &Source) -> Result<Detection, EngineError>;

    /// For a folder that is a clone of a GitHub repository: the repository,
    /// branch and commit, read from the folder without network or credentials.
    async fn github_remote(&self, _: &Source) -> Option<SourceGit> {
        None
    }
}
