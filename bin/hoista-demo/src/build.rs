//! Reading and building the demo's projects. Their code is small example
//! folders the seed writes under the state folder; a GitHub project reads its
//! folder of the same name, so nothing is ever cloned. Builds take a few
//! seconds and produce nothing, which is all a provider in the demo needs.

use hoista_server::application::{
    deploys::planning::Detection,
    engine::SourceGit,
    ports::{
        build::{BuildService, SourceInspector},
        catalog::EngineError,
        providers::Artifact,
    },
};
use hoista_server::domain::{
    Secret,
    model::{Plan, Source},
    provenance::SourceObservation,
};
use hoista_server::infrastructure::inspection::{LocalInspector, detect};
use std::{path::PathBuf, time::Duration};

/// How long a demo build takes, so its step can be watched.
const BUILD_TIME: Duration = Duration::from_secs(4);

/// The folder holding the demo projects' code.
pub fn code_folder(state: &std::path::Path) -> PathBuf {
    state.join("code")
}

/// The folder a GitHub project's code is read from: its repository's name.
fn checkout(code: &std::path::Path, repository: &str) -> PathBuf {
    code.join(repository.rsplit('/').next().unwrap_or(repository))
}

pub struct DemoInspector {
    code: PathBuf,
}

impl DemoInspector {
    pub fn new(code: PathBuf) -> Self {
        Self { code }
    }
}

#[async_trait::async_trait]
impl SourceInspector for DemoInspector {
    async fn observe_source(&self, source: &Source) -> Result<SourceObservation, EngineError> {
        LocalInspector.observe_source(source).await
    }

    async fn detect(&self, source: &Source) -> Result<Detection, EngineError> {
        let root = match source {
            Source::Folder { path } => PathBuf::from(path),
            Source::Github { repository, .. } => checkout(&self.code, repository),
            Source::Unlinked => {
                return Err(EngineError::Invalid(
                    "Link this project to its code before deploying it.",
                ));
            }
        };
        tokio::task::spawn_blocking(move || detect(&root))
            .await
            .map_err(|_| EngineError::Store)?
    }

    async fn github_remote(&self, source: &Source) -> Option<SourceGit> {
        LocalInspector.github_remote(source).await
    }
}

pub struct DemoBuilder;

#[async_trait::async_trait]
impl BuildService for DemoBuilder {
    /// Values come only from what was set in Hoista, so a variable the
    /// project needs and nobody set stops the deploy, as it would for real.
    async fn secrets(&self, _plan: &Plan) -> Result<Vec<(String, Secret)>, EngineError> {
        Ok(Vec::new())
    }

    async fn build(
        &self,
        _plan: &Plan,
        _deploy_id: &str,
        secrets: Vec<(String, Secret)>,
    ) -> Result<Artifact, EngineError> {
        tokio::time::sleep(BUILD_TIME).await;
        Ok(Artifact {
            provenance: None,
            assets: Vec::new(),
            worker: None,
            worker_name: String::new(),
            worker_modules: Vec::new(),
            image: None,
            secrets,
        })
    }

    async fn cleanup(&self, _deploy_id: &str) -> Result<(), EngineError> {
        Ok(())
    }
}
