//! The remote and environment a command runs against, saved in `cli.json`
//! and overridden per command by `--remote` and `--environment`.
//!
//! `remotes.json` is shared with the desktop app, but the CLI signs in to each
//! remote separately and keeps its own key in the system keychain.

use crate::ui;
use anyhow::{Context as _, bail};
use hoista_server::{
    application::CredentialStore,
    client::LocalClient,
    domain::{Secret, model::Project},
    infrastructure::{credentials::SystemKeychain, paths::StatePaths},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

const MAX_PROJECTS: usize = 10_000;
const MAX_ID_BYTES: usize = 128;

/// The name that means this computer's own Hoista.
pub(crate) const HERE: &str = "this-computer";

/// One remote in the shared list. Its key is in the keychain, not here.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct RemoteRecord {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) added_at: String,
}

#[derive(Default, Serialize, Deserialize)]
struct RemoteBook {
    #[serde(default)]
    remotes: Vec<RemoteRecord>,
}

/// What `use` saved: the remote, and the environment for each Hoista.
#[derive(Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    remote: Option<String>,
    /// By remote id, or [`HERE`].
    #[serde(default)]
    environments: BTreeMap<String, String>,
}

/// A command's target and options.
pub(crate) struct Context {
    pub(crate) paths: StatePaths,
    remote_flag: Option<String>,
    environment_flag: Option<String>,
}

fn write_private(path: &std::path::Path, value: &impl Serialize) -> anyhow::Result<()> {
    use std::io::Write as _;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let next = path.with_extension("next");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut out = options.open(&next)?;
    out.write_all(&serde_json::to_vec_pretty(value)?)?;
    out.sync_all()?;
    std::fs::rename(next, path)?;
    Ok(())
}

fn read_json<T: Default + for<'de> Deserialize<'de>>(path: &std::path::Path) -> anyhow::Result<T> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("{} isn't readable", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(error.into()),
    }
}

/// The keychain entry for this command line's key to one remote.
pub(crate) fn key_name(id: &str) -> String {
    format!("hoist/remote/{id}")
}

impl Context {
    pub(crate) fn new(
        root: Option<std::path::PathBuf>,
        remote: Option<String>,
        environment: Option<String>,
    ) -> anyhow::Result<Self> {
        let context = Self {
            paths: StatePaths::new(root)?,
            remote_flag: remote.filter(|value| !value.is_empty()),
            environment_flag: environment.filter(|value| !value.is_empty()),
        };
        context.adopt_old_remote()?;
        Ok(context)
    }

    fn book_file(&self) -> std::path::PathBuf {
        self.paths.root().join("remotes.json")
    }

    fn saved_file(&self) -> std::path::PathBuf {
        self.paths.root().join("cli.json")
    }

    pub(crate) fn remotes(&self) -> anyhow::Result<Vec<RemoteRecord>> {
        Ok(read_json::<RemoteBook>(&self.book_file())?.remotes)
    }

    pub(crate) fn save_remotes(&self, remotes: Vec<RemoteRecord>) -> anyhow::Result<()> {
        write_private(&self.book_file(), &RemoteBook { remotes })
    }

    /// Before there could be several remotes, the one remote was kept in
    /// `server.json`, its key under `hoist/server-key`. Moves it to the list.
    fn adopt_old_remote(&self) -> anyhow::Result<()> {
        let old = self.paths.root().join("server.json");
        let Ok(bytes) = std::fs::read(&old) else {
            return Ok(());
        };
        let value: Value = serde_json::from_slice(&bytes).unwrap_or_default();
        let Some(url) = value["url"].as_str() else {
            return Ok(());
        };
        let mut remotes = self.remotes()?;
        let id = match remotes.iter().find(|remote| remote.url == url) {
            Some(remote) => remote.id.clone(),
            None => {
                let record = RemoteRecord {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: suggested_name(url, &remotes),
                    url: url.to_owned(),
                    added_at: now(),
                };
                let id = record.id.clone();
                remotes.push(record);
                self.save_remotes(remotes)?;
                id
            }
        };
        if let Ok(key) = SystemKeychain.get("hoist/server-key") {
            SystemKeychain
                .set(&key_name(&id), &key)
                .map_err(|error| anyhow::anyhow!("Couldn't move the remote's key: {error}"))?;
            let _ = SystemKeychain.delete("hoist/server-key");
        }
        let _ = std::fs::remove_file(old);
        Ok(())
    }

    fn saved(&self) -> anyhow::Result<Saved> {
        read_json(&self.saved_file())
    }

    fn save(&self, saved: &Saved) -> anyhow::Result<()> {
        write_private(&self.saved_file(), saved)
    }

    /// Finds a remote by name, id or address.
    pub(crate) fn remote_named(&self, wanted: &str) -> anyhow::Result<RemoteRecord> {
        let remotes = self.remotes()?;
        remotes
            .iter()
            .find(|remote| {
                remote.name.eq_ignore_ascii_case(wanted)
                    || remote.id == wanted
                    || remote.url == wanted.trim_end_matches('/')
            })
            .cloned()
            .with_context(|| {
                if remotes.is_empty() {
                    format!(
                        "There's no remote called {}. Add one with hoist remote add NAME URL.",
                        ui::clean(wanted)
                    )
                } else {
                    format!(
                        "There's no remote called {}. Your remotes: {}.",
                        ui::clean(wanted),
                        remotes
                            .iter()
                            .map(|remote| remote.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            })
    }

    /// The remote this command talks to, or `None` for this computer.
    pub(crate) fn target(&self) -> anyhow::Result<Option<RemoteRecord>> {
        let wanted = match &self.remote_flag {
            Some(flag) => Some(flag.clone()),
            None => self.saved()?.remote,
        };
        match wanted.as_deref() {
            None | Some(HERE) => Ok(None),
            Some(name) => match self.remote_named(name) {
                Ok(remote) => Ok(Some(remote)),
                // A saved choice whose remote was removed falls back to here.
                Err(_) if self.remote_flag.is_none() => Ok(None),
                Err(error) => Err(error),
            },
        }
    }

    /// The name of the Hoista this command talks to, for messages.
    pub(crate) fn target_name(&self) -> anyhow::Result<String> {
        Ok(self
            .target()?
            .map_or_else(|| HERE.to_owned(), |remote| remote.name))
    }

    pub(crate) fn use_remote(&self, name: &str) -> anyhow::Result<String> {
        let mut saved = self.saved()?;
        let chosen = if name == HERE {
            None
        } else {
            Some(self.remote_named(name)?.name)
        };
        saved.remote.clone_from(&chosen);
        self.save(&saved)?;
        Ok(chosen.unwrap_or_else(|| HERE.to_owned()))
    }

    /// Forgets a removed remote's saved choices.
    pub(crate) fn forget_remote(&self, remote: &RemoteRecord) -> anyhow::Result<()> {
        let mut saved = self.saved()?;
        if saved.remote.as_deref() == Some(remote.name.as_str()) {
            saved.remote = None;
        }
        saved.environments.remove(&remote.id);
        self.save(&saved)
    }

    /// Renames a remote in the saved choice too.
    pub(crate) fn renamed_remote(&self, from: &str, to: &str) -> anyhow::Result<()> {
        let mut saved = self.saved()?;
        if saved.remote.as_deref() == Some(from) {
            saved.remote = Some(to.to_owned());
            self.save(&saved)?;
        }
        Ok(())
    }

    fn target_key(&self) -> anyhow::Result<String> {
        Ok(self
            .target()?
            .map_or_else(|| HERE.to_owned(), |remote| remote.id))
    }

    /// A client for the target: this computer's Hoista (started if
    /// needed) or the remote, with this computer's key for it.
    pub(crate) async fn client(&self) -> anyhow::Result<LocalClient> {
        match self.target()? {
            None => self.local().await,
            Some(remote) => remote_client(&self.paths, &remote),
        }
    }

    /// This computer's Hoista, whatever the target is.
    pub(crate) async fn local(&self) -> anyhow::Result<LocalClient> {
        let _daemon = crate::bootstrap::ensure(&self.paths).await?;
        Ok(LocalClient::new(&self.paths)?)
    }

    /// The environment this command works in.
    pub(crate) async fn environment(&self, client: &LocalClient) -> anyhow::Result<Environment> {
        let all = environments(client).await?;
        let wanted = match &self.environment_flag {
            Some(flag) => Some(flag.clone()),
            None => self.saved()?.environments.get(&self.target_key()?).cloned(),
        };
        match wanted {
            None => all
                .iter()
                .find(|environment| environment.id == "env_default")
                .or_else(|| all.first())
                .cloned()
                .context("This Hoista has no environments."),
            Some(wanted) => match find_environment(&all, &wanted) {
                Ok(found) => Ok(found),
                // A saved choice that was removed falls back to the default.
                Err(_) if self.environment_flag.is_none() => all
                    .iter()
                    .find(|environment| environment.id == "env_default")
                    .cloned()
                    .context("This Hoista has no environments."),
                Err(error) => Err(error),
            },
        }
    }

    pub(crate) fn use_environment(&self, environment: &Environment) -> anyhow::Result<()> {
        let mut saved = self.saved()?;
        saved
            .environments
            .insert(self.target_key()?, environment.name.clone());
        self.save(&saved)
    }
}

/// A client for one remote, with this command line's key for it.
pub(crate) fn remote_client(
    paths: &StatePaths,
    remote: &RemoteRecord,
) -> anyhow::Result<LocalClient> {
    let key: Secret = SystemKeychain.get(&key_name(&remote.id)).map_err(|_| {
        anyhow::anyhow!(
            "This computer isn't signed in to {}. Run hoist remote login {}.",
            ui::clean(&remote.name),
            ui::clean(&remote.name)
        )
    })?;
    Ok(LocalClient::on_server(paths, &remote.url, key)?)
}

pub(crate) fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// A short name for a remote from its address: `hoista.example.com` → `hoista`.
pub(crate) fn suggested_name(url: &str, taken: &[RemoteRecord]) -> String {
    let host = url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned))
        .unwrap_or_default();
    let base = if matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]") {
        "docker".to_owned()
    } else {
        host.split('.')
            .next()
            .filter(|part| !part.is_empty())
            .unwrap_or("remote")
            .to_owned()
    };
    let mut name = base.clone();
    let mut count = 2;
    while taken.iter().any(|remote| remote.name == name) {
        name = format!("{base}-{count}");
        count += 1;
    }
    name
}

/// A Hoista environment: production, staging, or the default one.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Environment {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) created_at: String,
}

pub(crate) async fn environments(client: &LocalClient) -> anyhow::Result<Vec<Environment>> {
    let value = client.get_json("/environments").await?;
    let items = value.get("items").cloned().unwrap_or(value);
    serde_json::from_value(items).context("Hoista's environments didn't read")
}

pub(crate) fn find_environment(all: &[Environment], wanted: &str) -> anyhow::Result<Environment> {
    all.iter()
        .find(|environment| {
            environment.name.eq_ignore_ascii_case(wanted) || environment.id == wanted
        })
        .cloned()
        .with_context(|| {
            format!(
                "There's no environment called {}. Environments: {}.",
                ui::clean(wanted),
                all.iter()
                    .map(|environment| environment.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

/// Every project, across every page.
pub(crate) async fn projects(client: &LocalClient) -> anyhow::Result<Vec<Project>> {
    let mut values = vec![];
    let mut path = "/projects".to_owned();
    loop {
        let page = client.get_json(&path).await?;
        let items: Vec<Project> = serde_json::from_value(page["items"].clone())
            .context("Hoista's project list didn't read")?;
        values.extend(items);
        if values.len() > MAX_PROJECTS {
            bail!("There are more than 10,000 projects; the command line stops there.");
        }
        let Some(cursor) = page["next_cursor"].as_str() else {
            break;
        };
        path = format!("/projects?{}", query(&[("cursor", cursor)]));
    }
    Ok(values)
}

/// A URL query string, encoded.
pub(crate) fn query(pairs: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs.iter().copied())
        .finish()
}

/// An API path with each segment encoded.
pub(crate) fn path(segments: &[&str]) -> String {
    let mut url = url::Url::parse("http://localhost").expect("a fixed URL parses");
    if let Ok(mut parts) = url.path_segments_mut() {
        parts.extend(segments);
    }
    url.path().to_owned()
}

/// Finds a project by id or name, preferring the current environment when
/// two environments use the same name.
pub(crate) fn pick_project<'a>(
    all: &'a [Project],
    wanted: &str,
    environment: &Environment,
) -> anyhow::Result<&'a Project> {
    if let Some(project) = all.iter().find(|project| project.id == wanted) {
        return Ok(project);
    }
    let named: Vec<_> = all
        .iter()
        .filter(|project| project.name == wanted)
        .collect();
    let here: Vec<_> = named
        .iter()
        .copied()
        .filter(|project| project.environment_id == environment.id)
        .collect();
    match (here.as_slice(), named.as_slice()) {
        ([project], _) | ([], [project]) => Ok(project),
        ([], []) => bail!(
            "There's no project called {} in {}. Run hoist projects to see them.",
            ui::clean(wanted),
            environment.name
        ),
        _ => bail!(
            "More than one project is called {}. Pass --environment, or use its id from hoist projects --json.",
            ui::clean(wanted)
        ),
    }
}

/// The project a command is about: named, the one linked to this folder,
/// or picked from a list in a terminal.
pub(crate) async fn project(
    context: &Context,
    client: &LocalClient,
    wanted: Option<&str>,
    how: &str,
) -> anyhow::Result<Project> {
    let environment = context.environment(client).await?;
    let all = projects(client).await?;
    if let Some(wanted) = wanted {
        return pick_project(&all, wanted, &environment).cloned();
    }
    // The project whose source is this folder, like `fly status` in an app folder.
    if let Ok(here) = std::env::current_dir().and_then(std::fs::canonicalize) {
        let linked: Vec<_> = all
            .iter()
            .filter(|project| {
                matches!(&project.source, hoista_server::domain::model::Source::Folder { path }
                    if std::path::Path::new(path) == here)
            })
            .collect();
        if let [project] = linked.as_slice() {
            return Ok((*project).clone());
        }
    }
    let in_environment: Vec<_> = all
        .iter()
        .filter(|project| project.environment_id == environment.id)
        .collect();
    if ui::can_ask() && !in_environment.is_empty() {
        let names: Vec<String> = in_environment
            .iter()
            .map(|project| ui::clean(&project.name))
            .collect();
        let chosen = ui::pick("Project", &names)?;
        return Ok(in_environment[chosen].clone());
    }
    if in_environment.is_empty() {
        bail!("There are no projects in {} yet.", environment.name);
    }
    ui::usage(how.to_owned())
}

/// Whether a value is an id Hoista gave out: letters, digits and hyphens.
pub(crate) fn check_id(id: &str, what: &str) -> anyhow::Result<()> {
    if id.is_empty()
        || id.len() > MAX_ID_BYTES
        || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return ui::usage(format!("That isn't {what}'s id."));
    }
    Ok(())
}

/// The resources of a project, from its overview.
pub(crate) async fn resources(
    client: &LocalClient,
    project: &Project,
) -> anyhow::Result<Vec<Value>> {
    let overview = client.get_json(&path(&["projects", &project.id])).await?;
    Ok(overview["resources"]
        .as_array()
        .cloned()
        .unwrap_or_default())
}

/// A resource by id, or by name within a project.
pub(crate) async fn resource(
    context: &Context,
    client: &LocalClient,
    wanted: Option<&str>,
    project: Option<&str>,
) -> anyhow::Result<Value> {
    if let Some(wanted) = wanted
        && project.is_none()
        && let Ok(found) = client.get_json(&path(&["resources", wanted])).await
    {
        return Ok(found);
    }
    let project = self::project(
        context,
        client,
        project,
        "Name the resource by id, or pass --project with its name.",
    )
    .await?;
    let all = resources(client, &project).await?;
    if let Some(wanted) = wanted {
        let found: Vec<_> = all
            .iter()
            .filter(|resource| resource["id"] == wanted || resource["name"] == wanted)
            .collect();
        return match found.as_slice() {
            [one] => Ok((*one).clone()),
            [] => bail!(
                "{} has no resource called {}.",
                ui::clean(&project.name),
                ui::clean(wanted)
            ),
            _ => bail!(
                "More than one resource is called {}. Use its id.",
                ui::clean(wanted)
            ),
        };
    }
    match all.as_slice() {
        [one] => Ok(one.clone()),
        [] => bail!("{} has no resources.", ui::clean(&project.name)),
        _ if ui::can_ask() => {
            let names: Vec<String> = all
                .iter()
                .map(|resource| {
                    format!(
                        "{} ({})",
                        ui::text(resource, "name"),
                        ui::text(resource, "provider")
                    )
                })
                .collect();
            Ok(all[ui::pick("Resource", &names)?].clone())
        }
        _ => ui::usage(format!(
            "{} has more than one resource. Pass its name or id.",
            ui::clean(&project.name)
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(id: &str) -> Environment {
        Environment {
            id: id.into(),
            name: id.into(),
            created_at: String::new(),
        }
    }

    fn project(id: &str, name: &str, environment: &str) -> Project {
        Project {
            id: id.into(),
            name: name.into(),
            source: hoista_server::domain::model::Source::Folder {
                path: "/fixture".into(),
            },
            mode: hoista_server::domain::model::Mode::Watching,
            created_at: "2026-11-02T00:00:00Z".into(),
            environment_id: environment.into(),
        }
    }

    #[test]
    fn a_name_in_two_environments_resolves_to_the_current_one() {
        let all = vec![
            project("one", "api", "env_default"),
            project("two", "api", "env_production"),
        ];
        assert_eq!(
            pick_project(&all, "api", &environment("env_production"))
                .unwrap()
                .id,
            "two"
        );
        assert_eq!(
            pick_project(&all, "one", &environment("env_production"))
                .unwrap()
                .id,
            "one"
        );
        assert!(pick_project(&all, "api", &environment("env_staging")).is_err());
        assert!(pick_project(&all, "web", &environment("env_default")).is_err());
    }

    #[test]
    fn remote_names_come_from_the_address_and_stay_unique() {
        let taken = vec![RemoteRecord {
            id: "a".into(),
            name: "hoista".into(),
            url: "https://hoista.example.com".into(),
            added_at: String::new(),
        }];
        assert_eq!(
            suggested_name("https://hoista.example.com", &taken),
            "hoista-2"
        );
        assert_eq!(
            suggested_name("https://office.example.com", &taken),
            "office"
        );
        assert_eq!(suggested_name("http://localhost:8787", &taken), "docker");
    }

    #[test]
    fn paths_encode_each_segment() {
        assert_eq!(
            path(&["projects", "a/b", "secrets"]),
            "/projects/a%2Fb/secrets"
        );
        assert_eq!(query(&[("q", "a&b")]), "q=a%26b");
    }

    #[test]
    fn saved_choices_live_beside_the_state_and_fall_back_when_gone() {
        let folder = tempfile::tempdir().unwrap();
        let context = Context::new(Some(folder.path().into()), None, None).unwrap();
        assert!(context.target().unwrap().is_none());
        context
            .save_remotes(vec![RemoteRecord {
                id: "id-1".into(),
                name: "office".into(),
                url: "https://office.example.com".into(),
                added_at: now(),
            }])
            .unwrap();
        assert_eq!(context.use_remote("office").unwrap(), "office");
        assert_eq!(context.target().unwrap().unwrap().id, "id-1");
        context.save_remotes(vec![]).unwrap();
        assert!(context.target().unwrap().is_none());
        let flagged = Context::new(Some(folder.path().into()), Some("gone".into()), None).unwrap();
        assert!(flagged.target().is_err());
        assert!(context.use_remote("gone").is_err());
        assert_eq!(context.use_remote(HERE).unwrap(), HERE);
    }
}
