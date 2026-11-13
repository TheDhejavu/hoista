//! The project an agent's folder belongs to, and plans made inside a project.
//!
//! A coding tool runs in a folder. That folder belongs to one project when it
//! names it in `.hoista/project.json`, when it is (or is inside) the project's
//! source folder, or when it is a clone of the project's repository. Tools
//! about one project use it when the agent leaves `project_id` out, so the
//! agent doesn't need to remember an id across sessions.

use crate::engine::now;
use crate::projects::ProjectService;
use crate::{
    Provider,
    domain::model::{Mode, Plan, Project, ResourceKind, Source},
    engine::Actor,
    ports::catalog::EngineError,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The file in a folder that names its Hoista project.
pub const PROJECT_FILE: &str = ".hoista/project.json";

/// How a folder's project was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FoundBy {
    /// The folder's `.hoista/project.json` names it.
    ProjectFile,
    /// The folder is the project's source folder, or inside it.
    SourceFolder,
    /// The folder is a clone of the same GitHub repository as the project.
    Repository,
}

/// The project a folder belongs to.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct ActiveProject {
    /// The project's id.
    pub project_id: String,
    /// Its name.
    pub name: String,
    /// How it was found.
    pub found_by: FoundBy,
    /// The folder it was found for.
    pub folder: String,
}

#[derive(Deserialize)]
struct ProjectFile {
    project_id: String,
}

impl<'a> ProjectService<'a> {
    /// The project `folder` belongs to, if exactly one does.
    ///
    /// # Errors
    /// Reports unavailable persistent state.
    pub async fn active_project(self, folder: &Path) -> Result<Option<ActiveProject>, EngineError> {
        if !folder.is_absolute() {
            return Ok(None);
        }
        let projects = self.engine.accounts().all_projects().await?;
        let found = |project: &Project, found_by| ActiveProject {
            project_id: project.id.clone(),
            name: project.name.clone(),
            found_by,
            folder: folder.display().to_string(),
        };
        // Named in the folder, or a folder above it.
        if let Some(id) = named_in(folder)
            && let Some(project) = projects.iter().find(|project| project.id == id)
        {
            return Ok(Some(found(project, FoundBy::ProjectFile)));
        }
        // The project whose source folder holds it; the innermost wins.
        let here = Source::Folder {
            path: folder.display().to_string(),
        };
        if let Some(project) = projects
            .iter()
            .filter(|project| here.within(&project.source))
            .max_by_key(|project| match &project.source {
                Source::Folder { path } => path.len(),
                _ => 0,
            })
        {
            return Ok(Some(found(project, FoundBy::SourceFolder)));
        }
        // A clone of the project's repository, when only one project has it.
        let Some(remote) = self.engine.inspector.github_remote(&here).await else {
            return Ok(None);
        };
        let mut same = Vec::new();
        for project in &projects {
            let repository = match &project.source {
                Source::Github { repository, .. } => Some(repository.clone()),
                Source::Folder { .. } => self
                    .engine
                    .inspector
                    .github_remote(&project.source)
                    .await
                    .map(|git| git.repository),
                Source::Unlinked => None,
            };
            if repository.is_some_and(|name| name.eq_ignore_ascii_case(&remote.repository)) {
                same.push(project);
            }
        }
        Ok(match same.as_slice() {
            [project] => Some(found(project, FoundBy::Repository)),
            _ => None,
        })
    }

    /// Names `project_id` as the project of `folder`, in its
    /// `.hoista/project.json`, which is kept out of git: ids are this
    /// computer's own.
    ///
    /// # Errors
    /// Rejects a missing project or folder, or a file that can't be written.
    pub async fn use_project(
        self,
        actor: &Actor,
        project_id: &str,
        folder: &Path,
    ) -> Result<ActiveProject, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        let project = self
            .engine
            .repository
            .project(project_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if !folder.is_absolute() || !folder.is_dir() {
            return Err(EngineError::Invalid("The folder isn't available."));
        }
        let root = repository_root(folder).unwrap_or_else(|| folder.to_path_buf());
        let dir = root.join(".hoista");
        let written = std::fs::create_dir_all(&dir).and_then(|()| {
            std::fs::write(dir.join(".gitignore"), "*\n")?;
            std::fs::write(
                dir.join("project.json"),
                serde_json::to_string_pretty(&serde_json::json!({
                    "project_id": project.id,
                    "name": project.name,
                    "written_at": now(),
                }))
                .map_err(std::io::Error::other)?
                    + "\n",
            )
        });
        if written.is_err() {
            return Err(EngineError::Invalid(
                "Hoista couldn't write .hoista/project.json in that folder.",
            ));
        }
        self.engine
            .repository
            .event(
                actor.id(),
                Some(&project.id),
                "project.folder_bound",
                &format!("Named this project in {}.", root.display()),
            )
            .await?;
        Ok(ActiveProject {
            project_id: project.id,
            name: project.name,
            found_by: FoundBy::ProjectFile,
            folder: root.display().to_string(),
        })
    }

    /// A deploy plan for code inside a project: its folder or a folder in it
    /// (one app of a monorepo). It uses the project's own database rather
    /// than proposing another.
    ///
    /// # Errors
    /// Rejects a source outside the project, or anything `plan` rejects.
    pub async fn plan_in_project(
        self,
        actor: &Actor,
        project_id: &str,
        source: Source,
        name: &str,
        pinned: Option<Provider>,
    ) -> Result<Plan, EngineError> {
        self.engine.authorize(actor, "apps:write").await?;
        let project = self
            .engine
            .repository
            .project(project_id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if project.mode != Mode::Managed {
            return Err(EngineError::Watching);
        }
        match &project.source {
            Source::Unlinked => {
                return Err(EngineError::Invalid(
                    "This project has no source yet. Link its folder with link_folder, then plan from it or a folder inside it.",
                ));
            }
            root if !source.within(root) => {
                return Err(EngineError::Invalid(
                    "Plan from the project's source folder or a folder inside it (read its source with get_project).",
                ));
            }
            _ => {}
        }
        let resources: Vec<_> = self
            .engine
            .projects()
            .all_resources()
            .await?
            .into_iter()
            .filter(|resource| resource.project_id.as_deref() == Some(project_id))
            .collect();
        // Deploying again keeps the provider it runs on. A database often shares
        // the app's name, so only something that runs code counts.
        let existing = resources
            .iter()
            .find(|resource| {
                resource.name == name && resource.mode == Mode::Managed && resource.kind.runs_code()
            })
            .map(|resource| resource.provider);
        let mut plan = self
            .engine
            .plan_unsaved(source, name, pinned, existing)
            .await?;
        let database = resources
            .iter()
            .find(|resource| resource.kind == ResourceKind::Database);
        if let (Some(index), Some(database)) = (
            plan.resources
                .iter()
                .position(|resource| resource.kind == ResourceKind::Database),
            database,
        ) {
            let planned = plan.resources[index].name.clone();
            let can_hold = self
                .engine
                .adapter(database.provider)
                .is_ok_and(|adapter| adapter.capabilities().project_database);
            if database.mode == Mode::Managed && can_hold {
                // Hoista made it: the deploy reuses it by name.
                let resource = &mut plan.resources[index];
                resource.name.clone_from(&database.name);
                resource.provider = database.provider;
                resource.account_id.clone_from(&database.account_id);
                for other in &mut plan.resources {
                    for dependency in &mut other.depends_on {
                        if *dependency == planned {
                            dependency.clone_from(&database.name);
                        }
                    }
                }
                plan.warnings.push(format!(
                    "Uses the project's database {} instead of making a new one.",
                    database.name
                ));
            } else {
                plan.resources.remove(index);
                for other in &mut plan.resources {
                    other.depends_on.retain(|dependency| *dependency != planned);
                }
                if plan.prisma_migration.take().is_some() {
                    plan.warnings.push(format!(
                        "Prisma migrations aren't run on {}, a database Hoista doesn't manage. Run them yourself.",
                        database.name
                    ));
                }
                plan.warnings.push(format!(
                    "Uses the project's database {} on {} instead of making a new one. If the code reads DATABASE_URL, set it on the new resource with set_secret.",
                    database.name,
                    database.provider.display_name()
                ));
            }
        }
        self.engine.save_plan(actor, plan).await
    }
}

/// The project id a folder's `.hoista/project.json` names, looking in the
/// folder and the folders above it up to the repository's root.
fn named_in(folder: &Path) -> Option<String> {
    let stop = repository_root(folder);
    for dir in folder.ancestors() {
        if let Ok(text) = std::fs::read_to_string(dir.join(PROJECT_FILE))
            && text.len() <= 4096
            && let Ok(file) = serde_json::from_str::<ProjectFile>(&text)
            && !file.project_id.is_empty()
            && file.project_id.len() <= 128
        {
            return Some(file.project_id);
        }
        if stop.as_deref() == Some(dir) {
            break;
        }
    }
    None
}

/// The git repository a folder is in: the nearest folder above it with `.git`.
fn repository_root(folder: &Path) -> Option<PathBuf> {
    folder
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf)
}
