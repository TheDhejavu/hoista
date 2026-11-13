//! Projects: listing, status, importing what already runs on your providers,
//! linking a source, the map, and environments.

use crate::{
    context::{self, Context, Environment},
    ui,
};
use anyhow::{Context as _, bail};
use clap::{Args, Subcommand};
use hoista_server::{
    client::LocalClient,
    domain::model::{Project, valid_slug},
};
use serde_json::{Value, json};
use std::path::PathBuf;

/// A project name is one DNS label.
const MAX_SLUG_CHARS: usize = 63;

// hoist projects

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct ProjectsArgs {
    #[command(subcommand)]
    command: Option<ProjectsCommand>,
    /// List projects in every environment.
    #[arg(long)]
    all: bool,
}

#[derive(Subcommand)]
enum ProjectsCommand {
    /// Rename a project in Hoista. Nothing changes at the provider.
    Rename {
        /// Project name or id.
        project: String,
        /// The new name: lowercase letters, digits and hyphens.
        name: String,
    },
    /// Move a project to another environment.
    Move {
        /// Project name or id.
        project: Option<String>,
        /// The environment to move it to.
        #[arg(long, value_name = "ENVIRONMENT")]
        to: Option<String>,
    },
    /// Merge a watched project into another one: its resources move there.
    Merge {
        /// The project to merge away.
        project: Option<String>,
        /// The project that keeps the resources.
        #[arg(long, value_name = "PROJECT")]
        into: Option<String>,
    },
    /// Let Hoista deploy and change a project.
    Manage {
        /// Project name or id.
        project: Option<String>,
    },
    /// Only watch a project: Hoista shows it but doesn't change it.
    Watch {
        /// Project name or id.
        project: Option<String>,
    },
    /// Stop watching a project. It keeps running at its provider.
    Remove {
        /// Project name or id.
        project: Option<String>,
    },
    /// Delete a project at its providers, after saving copies of its databases.
    Delete(DeleteArgs),
}

#[derive(Args)]
pub(crate) struct DeleteArgs {
    /// Project name or id.
    #[arg(value_name = "PROJECT")]
    pub(crate) target: Option<String>,
    /// Show what would be deleted, and delete nothing.
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// The name, typed to confirm, for scripts.
    #[arg(long, value_name = "NAME")]
    pub(crate) confirm: Option<String>,
    /// Delete only if what would be deleted still matches this fingerprint from --dry-run.
    #[arg(long, value_name = "HASH")]
    pub(crate) fingerprint: Option<String>,
    /// Don't save copies of the project's databases first.
    #[arg(long)]
    pub(crate) skip_backup: bool,
}

/// Every project summary in an environment (or all), across every page.
async fn summaries(
    client: &LocalClient,
    environment: Option<&Environment>,
) -> anyhow::Result<Vec<Value>> {
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..200 {
        let mut pairs = vec![];
        if let Some(environment) = environment {
            pairs.push(("environment", environment.id.as_str()));
        }
        if let Some(cursor) = &cursor {
            pairs.push(("cursor", cursor.as_str()));
        }
        let page = client
            .get_json(&format!("/projects/summaries?{}", context::query(&pairs)))
            .await?;
        items.extend(page["items"].as_array().cloned().unwrap_or_default());
        match page["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => return Ok(items),
        }
    }
    bail!("The project list didn't end.")
}

fn providers_of(resources: &Value) -> String {
    let mut providers: Vec<String> = resources
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| ui::text(item, "provider"))
                .collect()
        })
        .unwrap_or_default();
    providers.sort();
    providers.dedup();
    if providers.is_empty() {
        "-".into()
    } else {
        providers.join(", ")
    }
}

fn last_deploy(overview: &Value) -> String {
    let deploy = &overview["last_deploy"];
    if deploy.is_null() {
        return "-".into();
    }
    format!(
        "{} {}",
        ui::text(deploy, "status"),
        ui::when(deploy, "started_at")
    )
}

pub(crate) async fn projects(context: &Context, args: ProjectsArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    let Some(command) = args.command else {
        let environment = context.environment(&client).await?;
        let names: std::collections::HashMap<String, String> = context::environments(&client)
            .await?
            .into_iter()
            .map(|environment| (environment.id, environment.name))
            .collect();
        let items = summaries(&client, (!args.all).then_some(&environment)).await?;
        let empty = if args.all {
            "No projects yet. Bring some in with hoist import, or deploy a folder with hoist deploy.".to_owned()
        } else {
            format!(
                "No projects in {} yet. Bring some in with hoist import, or deploy a folder with hoist deploy.",
                environment.name
            )
        };
        return ui::list(
            &Value::Array(items),
            &empty,
            |item| {
                let mut row = vec![
                    ui::text(item, "project.name"),
                    ui::text(item, "status"),
                    ui::text(item, "project.mode"),
                    providers_of(&item["resources"]),
                    last_deploy(item),
                ];
                if args.all {
                    row.insert(
                        1,
                        names
                            .get(
                                item["project"]["environment_id"]
                                    .as_str()
                                    .unwrap_or_default(),
                            )
                            .cloned()
                            .unwrap_or_else(|| "-".into()),
                    );
                }
                row
            },
            if args.all {
                &[
                    "NAME",
                    "ENVIRONMENT",
                    "STATUS",
                    "MODE",
                    "PROVIDERS",
                    "LAST DEPLOY",
                ]
            } else {
                &["NAME", "STATUS", "MODE", "PROVIDERS", "LAST DEPLOY"]
            },
        );
    };
    match command {
        ProjectsCommand::Rename { project, name } => {
            if !valid_slug(&name) {
                return ui::usage("Use a name of lowercase letters, digits and hyphens.");
            }
            let project = context::project(context, &client, Some(&project), "").await?;
            let renamed = client
                .put_json(
                    &context::path(&["projects", &project.id, "name"]),
                    &json!({"name": name}),
                )
                .await?;
            ui::show(&renamed, |_| {
                ui::done(&format!("Renamed {} to {name}.", ui::clean(&project.name)))
            })
        }
        ProjectsCommand::Move { project, to } => {
            let project = context::project(
                context,
                &client,
                project.as_deref(),
                "Name the project: hoist projects move PROJECT --to ENVIRONMENT.",
            )
            .await?;
            let all = context::environments(&client).await?;
            let target = match to {
                Some(to) => context::find_environment(&all, &to)?,
                None if ui::can_ask() => {
                    let others: Vec<_> = all
                        .iter()
                        .filter(|environment| environment.id != project.environment_id)
                        .collect();
                    if others.is_empty() {
                        bail!(
                            "There's no other environment. Add one with hoist environments add NAME."
                        );
                    }
                    let names: Vec<String> = others
                        .iter()
                        .map(|environment| environment.name.clone())
                        .collect();
                    others[ui::pick("Move to", &names)?].clone()
                }
                None => {
                    return ui::usage("Say where: hoist projects move PROJECT --to ENVIRONMENT.");
                }
            };
            let moved = client
                .put_json(
                    &context::path(&["projects", &project.id, "environment"]),
                    &json!({"environment_id": target.id}),
                )
                .await?;
            ui::show(&moved, |_| {
                ui::done(&format!(
                    "Moved {} to {}.",
                    ui::clean(&project.name),
                    target.name
                ))
            })
        }
        ProjectsCommand::Merge { project, into } => {
            let from = context::project(
                context,
                &client,
                project.as_deref(),
                "Name the project: hoist projects merge PROJECT --into OTHER.",
            )
            .await?;
            let into = match into {
                Some(into) => context::project(context, &client, Some(&into), "").await?,
                None if ui::can_ask() => {
                    let environment = context.environment(&client).await?;
                    let all = context::projects(&client).await?;
                    let others: Vec<_> = all
                        .iter()
                        .filter(|project| {
                            project.id != from.id && project.environment_id == environment.id
                        })
                        .collect();
                    if others.is_empty() {
                        bail!(
                            "There's no other project in {} to merge into.",
                            environment.name
                        );
                    }
                    let names: Vec<String> = others
                        .iter()
                        .map(|project| ui::clean(&project.name))
                        .collect();
                    others[ui::pick("Merge into", &names)?].clone()
                }
                None => return ui::usage("Say which project keeps the resources: --into PROJECT."),
            };
            if !ui::confirm(
                &format!(
                    "Move {}'s resources into {} and remove {}? Nothing changes at the provider.",
                    ui::clean(&from.name),
                    ui::clean(&into.name),
                    ui::clean(&from.name)
                ),
                false,
            )? {
                return Ok(());
            }
            let merged = client
                .post_json(
                    &context::path(&["projects", &from.id, "merge"]),
                    &json!({"into": into.id}),
                )
                .await?;
            ui::show(&merged, |_| {
                ui::done(&format!("Merged into {}.", ui::clean(&into.name)))
            })
        }
        ProjectsCommand::Manage { project } => mode(context, &client, project, "managed").await,
        ProjectsCommand::Watch { project } => mode(context, &client, project, "watching").await,
        ProjectsCommand::Remove { project } => {
            let project = context::project(
                context,
                &client,
                project.as_deref(),
                "Name the project: hoist projects remove PROJECT --yes.",
            )
            .await?;
            if !ui::confirm(
                &format!(
                    "Stop watching {}? It keeps running at its provider; you can import it again later.",
                    ui::clean(&project.name)
                ),
                false,
            )? {
                return Ok(());
            }
            client
                .post_json(
                    &context::path(&["projects", &project.id, "stop-watching"]),
                    &json!({}),
                )
                .await?;
            ui::show(&json!({"removed": true, "project_id": project.id}), |_| {
                ui::done(&format!("Stopped watching {}.", ui::clean(&project.name)));
            })
        }
        ProjectsCommand::Delete(args) => {
            let project = context::project(
                context,
                &client,
                args.target.as_deref(),
                "Name the project: hoist projects delete PROJECT.",
            )
            .await?;
            delete(&client, "projects", &project.id, &project.name, &args).await
        }
    }
}

async fn mode(
    context: &Context,
    client: &LocalClient,
    project: Option<String>,
    mode: &str,
) -> anyhow::Result<()> {
    let project =
        context::project(context, client, project.as_deref(), "Name the project.").await?;
    let changed = client
        .put_json(
            &context::path(&["projects", &project.id, "mode"]),
            &json!({"mode": mode}),
        )
        .await?;
    ui::show(&changed, |_| {
        if mode == "managed" {
            ui::done(&format!(
                "Hoista now manages {}: it can deploy and change it.",
                ui::clean(&project.name)
            ));
        } else {
            ui::done(&format!(
                "Hoista now only watches {}.",
                ui::clean(&project.name)
            ));
        }
    })
}

/// Reviewed deletion, for a project or a resource: shows what goes, then
/// asks for the name typed exactly. The engine saves database copies first.
pub(crate) async fn delete(
    client: &LocalClient,
    family: &str,
    id: &str,
    name: &str,
    args: &DeleteArgs,
) -> anyhow::Result<()> {
    let preview = client
        .get_json(&context::path(&[family, id, "deletion-preview"]))
        .await?;
    if ui::mode().json && args.dry_run {
        println!("{}", serde_json::to_string_pretty(&preview)?);
        return Ok(());
    }
    if !ui::mode().json {
        println!("Deleting {} removes, at the provider:", ui::clean(name));
        for resource in preview["resources"].as_array().into_iter().flatten() {
            println!(
                "  {}  {} on {}",
                ui::text(resource, "name"),
                ui::text(resource, "kind"),
                ui::text(resource, "provider")
            );
        }
        for domain in preview["domains"].as_array().into_iter().flatten() {
            println!("  {}  domain", ui::text(domain, "hostname"));
        }
        let databases = preview["databases"].as_array().map_or(0, Vec::len);
        if databases > 0 {
            if args.skip_backup {
                ui::warn(&format!(
                    "{databases} databases are deleted without a copy."
                ));
            } else {
                println!("Hoista saves a copy of {databases} databases to this computer first.");
            }
        }
        for affected in preview["affected_resources"]
            .as_array()
            .into_iter()
            .flatten()
        {
            ui::warn(&format!(
                "{} uses it and may stop working.",
                ui::text(affected, "name")
            ));
        }
        println!("Fingerprint: {}", ui::text(&preview, "fingerprint"));
    }
    let blockers: Vec<String> = preview["blockers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|blocker| blocker.as_str().map(ui::clean))
        .collect();
    if !blockers.is_empty() {
        bail!("It can't be deleted yet: {}", blockers.join(" "));
    }
    if args.dry_run {
        ui::note("Nothing was deleted (--dry-run).");
        return Ok(());
    }
    let fingerprint = preview["fingerprint"]
        .as_str()
        .context("The preview had no fingerprint")?;
    if let Some(expected) = &args.fingerprint
        && expected != fingerprint
    {
        bail!("What would be deleted changed since that --dry-run. Look again with --dry-run.");
    }
    let confirmation = preview["confirmation_name"].as_str().unwrap_or(name);
    ui::type_to_confirm(confirmation, args.confirm.as_deref())?;
    ui::note("Deleting. Database copies are saved first; this can take a while.");
    let deleted = client
        .delete_json_with_body(
            &context::path(&[family, id]),
            &json!({
                "confirmation_name": confirmation,
                "fingerprint": fingerprint,
                "backup_databases": !args.skip_backup,
            }),
        )
        .await
        .context("The delete didn't confirm and may have partly happened. Check hoist projects and hoist db exports before trying again.")?;
    ui::show(&deleted, |deleted| {
        ui::done(&format!("Deleted {}.", ui::clean(name)));
        let archives = deleted["database_archives"].as_array().map_or(0, Vec::len);
        if archives > 0 {
            ui::note(&format!(
                "{archives} database copies saved. See them with hoist db exports."
            ));
        }
    })
}

// hoist status

#[derive(Args)]
pub(crate) struct StatusArgs {
    /// Project name or id. Default: the project linked to this folder.
    project: Option<String>,
}

fn source(source: &Value) -> String {
    match source["kind"].as_str() {
        Some("folder") => ui::text(source, "path"),
        Some("github") => format!("github.com/{}", ui::text(source, "repository")),
        _ => "not linked".into(),
    }
}

pub(crate) async fn status(context: &Context, args: StatusArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    let project = context::project(
        context,
        &client,
        args.project.as_deref(),
        "Name the project: hoist status PROJECT.",
    )
    .await?;
    let overview = client
        .get_json(&context::path(&["projects", &project.id]))
        .await?;
    let environments = context::environments(&client).await?;
    ui::show(&overview, |overview| {
        let environment = environments
            .iter()
            .find(|environment| environment.id == project.environment_id)
            .map_or_else(
                || project.environment_id.clone(),
                |environment| environment.name.clone(),
            );
        ui::fields(&[
            ("Project", ui::clean(&project.name)),
            ("Environment", environment),
            ("Status", ui::text(overview, "status")),
            ("Mode", ui::text(overview, "project.mode")),
            ("Source", source(&overview["project"]["source"])),
            ("Last deploy", last_deploy(overview)),
            ("Id", project.id.clone()),
        ]);
        println!();
        let mut table = ui::Table::new(&["RESOURCE", "KIND", "PROVIDER", "STATUS", "ADDRESS"]);
        for resource in overview["resources"].as_array().into_iter().flatten() {
            table.row(vec![
                ui::text(resource, "name"),
                ui::text(resource, "kind"),
                ui::text(resource, "provider"),
                ui::text(resource, "status"),
                ui::text(resource, "url"),
            ]);
        }
        if table.is_empty() {
            ui::note("No resources yet.");
        } else {
            table.print();
        }
    })
}

// hoist import

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct ImportArgs {
    #[command(subcommand)]
    command: Option<ImportCommand>,
    /// Resource ids to bring in. In a terminal, pick them from a list instead.
    resources: Vec<String>,
    /// Add them to this existing project.
    #[arg(long, value_name = "PROJECT", conflicts_with = "new")]
    project: Option<String>,
    /// Make a new project with this name.
    #[arg(long, value_name = "NAME")]
    new: Option<String>,
    /// Let Hoista deploy and change the new project. Default: watch only.
    #[arg(long, requires = "new")]
    manage: bool,
    /// List what Hoista found on your providers and hasn't brought in, and stop.
    #[arg(long)]
    list: bool,
    /// With --list: list the ones you hid.
    #[arg(long, requires = "list")]
    hidden: bool,
}

#[derive(Subcommand)]
enum ImportCommand {
    /// Hide something found so it stops being offered.
    Hide {
        /// Resource id from hoist import --list.
        resource: String,
    },
    /// Offer something you hid again.
    Unhide {
        /// Resource id from hoist import --list --hidden.
        resource: String,
    },
}

async fn candidates(client: &LocalClient, hidden: bool) -> anyhow::Result<Vec<Value>> {
    let mut items = Vec::new();
    let mut cursor = String::new();
    let mut seen = std::collections::HashSet::new();
    for _ in 0..1000 {
        let pairs = [
            ("ignored", if hidden { "true" } else { "false" }),
            ("cursor", cursor.as_str()),
        ];
        let page = client
            .get_json(&format!("/import/candidates?{}", context::query(&pairs)))
            .await?;
        items.extend(page["items"].as_array().cloned().unwrap_or_default());
        let Some(next) = page["next_cursor"].as_str() else {
            return Ok(items);
        };
        if !seen.insert(next.to_owned()) {
            bail!("The list of found resources repeated. Try again.");
        }
        cursor = next.to_owned();
    }
    bail!("Too much was found to list here. Narrow it down in the app.")
}

fn found_row(item: &Value) -> Vec<String> {
    vec![
        ui::text(item, "name"),
        ui::text(item, "kind"),
        ui::text(item, "provider"),
        ui::text(item, "group"),
        ui::text(item, "id"),
    ]
}

/// A project name from a provider's name for something: `My API` → `my-api`.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-')
        .chars()
        .take(MAX_SLUG_CHARS)
        .collect()
}

pub(crate) async fn import(context: &Context, args: ImportArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    if let Some(command) = args.command {
        let (id, hide) = match &command {
            ImportCommand::Hide { resource } => (resource, true),
            ImportCommand::Unhide { resource } => (resource, false),
        };
        context::check_id(id, "a resource")?;
        let changed = client
            .put_json(
                &context::path(&["resources", id, "ignored"]),
                &json!({"ignored": hide}),
            )
            .await?;
        return ui::show(&changed, |_| {
            ui::done(if hide {
                "Hidden. Find it again with hoist import --list --hidden."
            } else {
                "It's offered again."
            });
        });
    }
    let found = candidates(&client, args.hidden).await?;
    if args.list {
        return ui::list(
            &Value::Array(found),
            if args.hidden {
                "Nothing is hidden."
            } else {
                "Nothing new on your providers. Look again with hoist accounts refresh ACCOUNT."
            },
            found_row,
            &["NAME", "KIND", "PROVIDER", "PROVIDER PROJECT", "ID"],
        );
    }
    let chosen: Vec<Value> = if args.resources.is_empty() {
        if !ui::can_ask() {
            return ui::usage(
                "Pass resource ids from hoist import --list, and --new NAME or --project PROJECT.",
            );
        }
        if found.is_empty() {
            ui::note(
                "Nothing new on your providers. Look again with hoist accounts refresh ACCOUNT.",
            );
            return Ok(());
        }
        let labels: Vec<String> = found
            .iter()
            .map(|item| {
                format!(
                    "{}  {} on {}{}",
                    ui::text(item, "name"),
                    ui::text(item, "kind"),
                    ui::text(item, "provider"),
                    item["group"]
                        .as_str()
                        .map(|group| format!(" ({})", ui::clean(group)))
                        .unwrap_or_default()
                )
            })
            .collect();
        let picked = ui::pick_many("What to bring in", &labels)?;
        if picked.is_empty() {
            ui::note("Nothing picked.");
            return Ok(());
        }
        picked
            .into_iter()
            .map(|index| found[index].clone())
            .collect()
    } else {
        let mut chosen = Vec::new();
        for id in &args.resources {
            context::check_id(id, "a resource")?;
            chosen.push(
                found
                    .iter()
                    .find(|item| item["id"] == id.as_str())
                    .cloned()
                    .with_context(|| {
                        format!(
                            "{} isn't in what was found. See hoist import --list.",
                            ui::clean(id)
                        )
                    })?,
            );
        }
        chosen
    };
    let ids: Vec<&str> = chosen
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    let environment = context.environment(&client).await?;

    let existing = match (&args.project, &args.new) {
        (Some(project), _) => Some(context::project(context, &client, Some(project), "").await?),
        (None, Some(_)) => None,
        (None, None) if ui::can_ask() => {
            let all = context::projects(&client).await?;
            let here: Vec<&Project> = all
                .iter()
                .filter(|project| project.environment_id == environment.id)
                .collect();
            let mut options = vec!["A new project".to_owned()];
            options.extend(
                here.iter()
                    .map(|project| format!("Add to {}", ui::clean(&project.name))),
            );
            match ui::pick("Bring into", &options)? {
                0 => None,
                index => Some(here[index - 1].clone()),
            }
        }
        (None, None) => {
            return ui::usage("Say where: --new NAME for a new project, or --project PROJECT.");
        }
    };
    let done = if let Some(project) = existing {
        let added = client
            .post_json(
                "/import",
                &json!({"project_id": project.id, "resource_ids": ids}),
            )
            .await?;
        ui::show(&added, |_| {
            ui::done(&format!(
                "Added {} to {}.",
                ids.len(),
                ui::clean(&project.name)
            ))
        })?;
        return Ok(());
    } else {
        let suggested = chosen[0]["group"]
            .as_str()
            .or_else(|| chosen[0]["name"].as_str())
            .map(slug)
            .unwrap_or_default();
        let asked = args.new.is_none();
        let name = match args.new {
            Some(name) => name,
            None => ui::ask("Project name", Some(&suggested))?,
        };
        if !valid_slug(&name) {
            return ui::usage("Use a name of lowercase letters, digits and hyphens.");
        }
        let manage = args.manage
            || (asked
                && ui::can_ask()
                && ui::pick(
                    "Hoista should",
                    &[
                        "Only watch it".to_owned(),
                        "Manage it: deploy and change it".to_owned(),
                    ],
                )? == 1);
        let made = client
            .post_json(
                "/import/projects",
                &json!({
                    "name": name,
                    "resource_ids": ids,
                    "mode": if manage { "managed" } else { "watching" },
                    "environment_id": environment.id,
                }),
            )
            .await?;
        (made, name)
    };
    ui::show(&done.0, |_| {
        ui::done(&format!(
            "Made {} in {} with {} resources.",
            done.1,
            environment.name,
            ids.len()
        ));
        ui::note(&format!("See it with hoist status {}.", done.1));
    })
}

// hoist link

#[derive(Args)]
pub(crate) struct LinkArgs {
    /// Project name or id.
    project: Option<String>,
    /// The folder with the project's code. Default: this folder.
    #[arg(long, value_name = "PATH", conflicts_with = "repo")]
    folder: Option<PathBuf>,
    /// Link a GitHub repository instead, as OWNER/NAME.
    #[arg(long, value_name = "OWNER/NAME")]
    repo: Option<String>,
    /// With --repo: the branch to deploy. Default: the repository's default branch.
    #[arg(long, requires = "repo")]
    branch: Option<String>,
    /// With --repo: the GitHub account in Hoista that can read it.
    #[arg(long, value_name = "ACCOUNT", requires = "repo")]
    account: Option<String>,
}

pub(crate) async fn link(context: &Context, args: LinkArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    let project = context::project(
        context,
        &client,
        args.project.as_deref(),
        "Name the project: hoist link PROJECT.",
    )
    .await?;
    if let Some(repo) = args.repo {
        let github = |account: &Value| account["provider"] == "github";
        let account = crate::accounts::account(
            &client,
            args.account.as_deref(),
            Some(&github),
            "Pass --account with the GitHub account that can read it.",
        )
        .await?;
        let linked = client
            .put_json(
                &context::path(&["projects", &project.id, "repository"]),
                &json!({"account_id": account["id"], "repository": repo, "branch": args.branch}),
            )
            .await?;
        return ui::show(&linked, |_| {
            ui::done(&format!(
                "{} deploys from github.com/{} now.",
                ui::clean(&project.name),
                ui::clean(&repo)
            ));
        });
    }
    let folder = match args.folder {
        Some(folder) => folder,
        None => std::env::current_dir()?,
    };
    let folder = std::fs::canonicalize(&folder).context("That folder isn't there")?;
    if !folder.is_dir() {
        return ui::usage("--folder needs a folder.");
    }
    let path = folder.to_str().context("The folder's path isn't UTF-8")?;
    let linked = client
        .put_json(
            &context::path(&["projects", &project.id, "source"]),
            &json!({"source": {"kind": "folder", "path": path}}),
        )
        .await?;
    ui::show(&linked, |_| {
        ui::done(&format!(
            "{} deploys from {} now.",
            ui::clean(&project.name),
            ui::clean(path)
        ));
    })
}

// hoist map

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct MapArgs {
    #[command(subcommand)]
    command: Option<MapCommand>,
    /// Project name or id.
    project: Option<String>,
}

#[derive(Subcommand)]
enum MapCommand {
    /// Read a resource's connections from its provider again.
    Refresh {
        /// Resource name or id.
        resource: Option<String>,
        /// The resource's project, when naming it by name.
        #[arg(long)]
        project: Option<String>,
    },
}

pub(crate) async fn map(context: &Context, args: MapArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    if let Some(MapCommand::Refresh { resource, project }) = args.command {
        let resource =
            context::resource(context, &client, resource.as_deref(), project.as_deref()).await?;
        let id = ui::text(&resource, "id");
        let refreshed = client
            .post_json(
                &context::path(&["resources", &id, "relationships", "refresh"]),
                &json!({}),
            )
            .await?;
        return ui::show(&refreshed, |_| ui::done("Connections read again."));
    }
    let project = context::project(
        context,
        &client,
        args.project.as_deref(),
        "Name the project: hoist map PROJECT.",
    )
    .await?;
    let map = client
        .get_json(&context::path(&["projects", &project.id, "map"]))
        .await?;
    ui::show(&map, |map| {
        let nodes = map["nodes"].as_array().cloned().unwrap_or_default();
        let name = |id: &Value| {
            nodes
                .iter()
                .find(|node| node["id"] == *id)
                .map_or_else(|| ui::text(id, ""), |node| ui::text(node, "name"))
        };
        println!("{}", ui::clean(&project.name));
        for node in &nodes {
            println!(
                "  {}  {} on {}",
                ui::text(node, "name"),
                ui::text(node, "kind"),
                ui::text(node, "provider")
            );
        }
        let edges = map["edges"].as_array().cloned().unwrap_or_default();
        if !edges.is_empty() {
            println!();
            for edge in edges {
                println!(
                    "  {} → {}  via {}{}",
                    name(&edge["from_resource"]),
                    name(&edge["to_resource"]),
                    ui::text(&edge, "via"),
                    if edge["missing"] == true {
                        " (missing)"
                    } else {
                        ""
                    }
                );
            }
        }
    })
}

// hoist environments

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct EnvironmentsArgs {
    #[command(subcommand)]
    command: Option<EnvironmentsCommand>,
}

#[derive(Subcommand)]
enum EnvironmentsCommand {
    /// Add an environment.
    Add {
        /// Lowercase letters, digits and hyphens, such as staging.
        name: Option<String>,
    },
    /// Rename an environment.
    Rename {
        /// Its name now.
        environment: String,
        /// The new name.
        name: String,
    },
    /// Remove an empty environment.
    Remove {
        /// Environment name.
        environment: Option<String>,
    },
    /// Use an environment for the commands that follow.
    Use {
        /// Environment name.
        environment: Option<String>,
    },
}

pub(crate) async fn environments(context: &Context, args: EnvironmentsArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    let all = context::environments(&client).await?;
    let pick = |wanted: Option<String>, prompt: &str, how: &str| -> anyhow::Result<Environment> {
        match wanted {
            Some(wanted) => context::find_environment(&all, &wanted),
            None if ui::can_ask() => {
                let names: Vec<String> = all
                    .iter()
                    .map(|environment| environment.name.clone())
                    .collect();
                Ok(all[ui::pick(prompt, &names)?].clone())
            }
            None => ui::usage(how.to_owned()),
        }
    };
    match args.command {
        None => {
            let current = context.environment(&client).await?;
            let value = serde_json::to_value(&all)?;
            ui::show(&json!({"current": current.name, "items": value}), |_| {
                let mut table = ui::Table::new(&["", "NAME", "CREATED", "ID"]);
                for environment in &all {
                    table.row(vec![
                        if environment.id == current.id {
                            "*".into()
                        } else {
                            String::new()
                        },
                        environment.name.clone(),
                        ui::when(&json!({"at": environment.created_at}), "at"),
                        environment.id.clone(),
                    ]);
                }
                table.print();
            })
        }
        Some(EnvironmentsCommand::Add { name }) => {
            let name = ui::need(name, "Name", "Name it: hoist environments add NAME.")?;
            if !valid_slug(&name) {
                return ui::usage("Use a name of lowercase letters, digits and hyphens.");
            }
            let made = client
                .post_json("/environments", &json!({"name": name}))
                .await?;
            ui::show(&made, |_| {
                ui::done(&format!(
                    "Added {name}. Switch to it with hoist environments use {name}."
                ));
            })
        }
        Some(EnvironmentsCommand::Rename { environment, name }) => {
            let environment = context::find_environment(&all, &environment)?;
            if !valid_slug(&name) {
                return ui::usage("Use a name of lowercase letters, digits and hyphens.");
            }
            let renamed = client
                .patch_json(
                    &context::path(&["environments", &environment.id]),
                    &json!({"name": name}),
                )
                .await?;
            ui::show(&renamed, |_| {
                ui::done(&format!("Renamed {} to {name}.", environment.name))
            })
        }
        Some(EnvironmentsCommand::Remove { environment }) => {
            let environment = pick(
                environment,
                "Remove",
                "Name it: hoist environments remove NAME --yes.",
            )?;
            if !ui::confirm(
                &format!("Remove {}? It has to be empty.", environment.name),
                false,
            )? {
                return Ok(());
            }
            client
                .delete_json(&context::path(&["environments", &environment.id]))
                .await?;
            ui::show(&json!({"removed": environment.id}), |_| {
                ui::done(&format!("Removed {}.", environment.name))
            })
        }
        Some(EnvironmentsCommand::Use { environment }) => {
            let environment = pick(environment, "Use", "Name it: hoist environments use NAME.")?;
            context.use_environment(&environment)?;
            ui::show(&json!({"current": environment.name}), |_| {
                ui::done(&format!(
                    "Using {} on {}.",
                    environment.name,
                    context
                        .target_name()
                        .unwrap_or_else(|_| context::HERE.into())
                ));
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_names_become_project_names() {
        assert_eq!(slug("My API (prod)"), "my-api-prod");
        assert_eq!(slug("--web--"), "web");
        assert!(valid_slug(&slug("Status Page 9")));
    }
}
