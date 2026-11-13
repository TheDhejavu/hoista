//! Deploying: `deploy`, `plan`, `deploys`, `rollback`, `cancel` and `env`.
//!
//! A deploy is safe to retry: each one carries a request key, and sending
//! the same key again returns the first attempt instead of starting another.

use crate::{
    context::{self, Context},
    plans, ui,
};
use anyhow::{Context as _, bail};
use clap::{Args, Subcommand};
use hoista_server::{
    client::LocalClient,
    domain::model::{Mode, Plan, Project, Source, valid_env_name, valid_slug},
};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};
use zeroize::Zeroize;

/// How often a deploy being followed is checked.
const FOLLOW_POLL: Duration = Duration::from_secs(2);
const MAX_VALUES: usize = 64;
const MAX_VALUES_BYTES: usize = 64 * 1024;

// hoist deploy

#[derive(Args)]
pub(crate) struct DeployArgs {
    /// The folder to deploy. Default: this folder.
    folder: Option<PathBuf>,
    /// The project to deploy to. Default: the one linked to the folder, or a new one named after it.
    #[arg(long, short = 'p', value_name = "NAME")]
    project: Option<String>,
    /// A note saved with the deploy, like a commit message.
    #[arg(long, short = 'm', value_name = "TEXT")]
    message: Option<String>,
    /// Start the deploy and return without waiting for it to finish.
    #[arg(long)]
    detach: bool,
    /// Deploy a plan you reviewed with hoist plan, instead of planning again.
    #[arg(long, value_name = "PLAN", requires = "project")]
    plan: Option<String>,
    /// With --plan: deploy only if the plan is still this revision.
    #[arg(long, value_name = "N", requires = "plan", value_parser = clap::value_parser!(i64).range(0..=9_007_199_254_740_991))]
    plan_revision: Option<i64>,
    /// A UUID that makes retrying safe: the same key never deploys twice.
    #[arg(long, value_name = "UUID")]
    request_key: Option<String>,
}

fn request_key(given: Option<String>) -> anyhow::Result<String> {
    match given {
        Some(key) => uuid::Uuid::parse_str(&key)
            .map(|key| key.to_string())
            .or_else(|_| ui::usage("--request-key needs a UUID.")),
        None => Ok(uuid::Uuid::new_v4().to_string()),
    }
}

fn folder_source(folder: Option<PathBuf>) -> anyhow::Result<(PathBuf, Source)> {
    let folder = match folder {
        Some(folder) => folder,
        None => std::env::current_dir()?,
    };
    let path = std::fs::canonicalize(&folder).context("That folder isn't there")?;
    if !path.is_dir() {
        return ui::usage("Give a folder to deploy.");
    }
    let source = Source::Folder {
        path: path
            .to_str()
            .context("The folder's path isn't UTF-8")?
            .into(),
    };
    Ok((path, source))
}

/// Prints what a plan will put where.
fn show_plan(plan: &Plan, accounts: &[Value]) {
    let label = |id: &str| {
        accounts
            .iter()
            .find(|account| account["id"] == id)
            .map_or_else(|| id.to_owned(), |account| ui::text(account, "label"))
    };
    ui::fields(&[
        ("Framework", ui::clean(&plan.framework)),
        (
            "Build",
            if plan.build_command.is_empty() {
                "-".into()
            } else {
                ui::clean(&plan.build_command.join(" "))
            },
        ),
        ("Plan", format!("{} (revision {})", plan.id, plan.revision)),
    ]);
    println!();
    let mut table = ui::Table::new(&["RESOURCE", "KIND", "PROVIDER", "ACCOUNT", "REGION"]);
    for resource in &plan.resources {
        table.row(vec![
            ui::clean(&resource.name),
            format!("{:?}", resource.kind).to_lowercase(),
            resource.provider.slug().to_owned(),
            ui::clean(&label(&resource.account_id)),
            resource
                .region
                .as_deref()
                .map_or_else(|| "-".into(), ui::clean),
        ]);
    }
    table.print();
    if !plan.env_names.is_empty() {
        println!(
            "\nVariables it needs: {}",
            ui::clean(&plan.env_names.join(", "))
        );
    }
    for warning in &plan.warnings {
        ui::warn(&ui::clean(warning));
    }
}

async fn all_accounts(client: &LocalClient) -> Vec<Value> {
    client
        .get_json("/accounts")
        .await
        .ok()
        .and_then(|page| page["items"].as_array().cloned())
        .unwrap_or_default()
}

pub(crate) async fn deploy(context: &Context, args: DeployArgs) -> anyhow::Result<bool> {
    let client = context.client().await?;
    let environment = context.environment(&client).await?;
    let key = request_key(args.request_key)?;

    let (project, plan) = if let Some(plan_id) = &args.plan {
        let project = context::project(context, &client, args.project.as_deref(), "").await?;
        let plan = plans::show(&client, plan_id).await?;
        (project, plan)
    } else {
        let (path, source) = folder_source(args.folder)?;
        let catalog = context::projects(&client).await?;
        let existing: Vec<&Project> = catalog
            .iter()
            .filter(|project| {
                project.environment_id == environment.id
                    && match &args.project {
                        Some(wanted) => project.name == *wanted || project.id == *wanted,
                        None => project.source == source,
                    }
            })
            .collect();
        if existing.len() > 1 {
            return ui::usage(
                "More than one project deploys from this folder. Pass --project NAME.",
            );
        }
        let existing = existing.first().copied();
        let name = args
            .project
            .clone()
            .or_else(|| existing.map(|project| project.name.clone()))
            .or_else(|| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_lowercase)
            })
            .context("Pass --project NAME for this folder")?;
        if existing.is_none() && !valid_slug(&name) {
            return ui::usage(
                "Pass --project with a name of lowercase letters, digits and hyphens.",
            );
        }
        if let Some(project) = existing {
            if project.mode != Mode::Managed {
                bail!(
                    "Hoista only watches {}. Let it deploy with hoist projects manage {}.",
                    ui::clean(&project.name),
                    ui::clean(&project.name)
                );
            }
            if project.source != source {
                bail!(
                    "{} deploys from another folder. Link this one first with hoist link {} --folder {}.",
                    ui::clean(&project.name),
                    ui::clean(&project.name),
                    ui::clean(&path.display().to_string())
                );
            }
        }
        let resources: Vec<Value> = match existing {
            Some(project) => context::resources(&client, project).await?,
            None => vec![],
        };
        // Planned within its project, a deploy stays on the provider its code
        // runs on and uses the project's own database rather than a new one.
        ui::note("Planning");
        let plan: Plan = serde_json::from_value(
            client
                .post_json(
                    "/plans",
                    &json!({
                        "source": source,
                        "name": name,
                        "pinned": null,
                        "project_id": existing.map(|project| project.id.as_str()),
                    }),
                )
                .await?,
        )
        .context("The plan didn't read")?;
        for planned in &plan.resources {
            let kind = serde_json::to_value(planned.kind)?;
            if let Some(current) = resources.iter().find(|resource| {
                resource["name"] == planned.name.as_str() && resource["kind"] == kind
            }) && (current["provider"] != planned.provider.slug()
                || current["account_id"] != planned.account_id.as_str())
            {
                bail!(
                    "The plan put {} on another account. Pick its account with hoist plan set {} {} --account {}, then hoist deploy --project {} --plan {}.",
                    ui::clean(&planned.name),
                    plan.id,
                    ui::clean(&planned.name),
                    ui::text(current, "account_id"),
                    ui::clean(&name),
                    plan.id
                );
            }
        }
        let project = match existing {
            Some(project) => project.clone(),
            None => {
                if !ui::mode().json {
                    println!(
                        "This makes a new project, {}, in {}:\n",
                        ui::clean(&name),
                        environment.name
                    );
                    show_plan(&plan, &all_accounts(&client).await);
                    println!();
                }
                if ui::can_ask() && !ui::confirm("Deploy it?", true)? {
                    ui::note(&format!(
                        "Nothing deployed. Change the plan with hoist plan set {}.",
                        plan.id
                    ));
                    return Ok(true);
                }
                serde_json::from_value(
                    client
                        .post_json(
                            "/projects",
                            &json!({
                                "name": name,
                                "source": source,
                                "mode": "managed",
                                "environment_id": environment.id,
                            }),
                        )
                        .await?,
                )?
            }
        };
        (project, plan)
    };
    let revision = args.plan_revision.unwrap_or(plan.revision);
    let started = client
        .post_json(
            &context::path(&["projects", &project.id, "deploys"]),
            &json!({
                "request_key": key,
                "plan_id": plan.id,
                "expected_plan_revision": revision,
                "channel": "cli",
                "title": args.message,
            }),
        )
        .await
        .with_context(|| {
            format!(
                "The deploy didn't start, or its answer was lost. Retry safely with the same key: hoist deploy --project {} --plan {} --plan-revision {revision} --request-key {key}",
                project.name, plan.id
            )
        })?;
    let id = started["id"]
        .as_str()
        .context("The deploy didn't say its id")?
        .to_owned();
    let mut started = started;
    started["request_key"] = json!(key);
    started["plan_id"] = json!(plan.id);
    started["plan_revision"] = json!(revision);
    if args.detach || ui::mode().json {
        ui::show(&started, |_| {
            ui::done(&format!(
                "Deploy {id} started. Follow it with hoist deploys show {id}."
            ));
        })?;
        return Ok(true);
    }
    ui::note(&format!(
        "Deploying {} (deploy {id})",
        ui::clean(&project.name)
    ));
    follow(&client, &id).await
}

/// Prints a deploy's steps as they change, until it finishes. Ctrl-C stops
/// watching; the deploy keeps going.
async fn follow(client: &LocalClient, id: &str) -> anyhow::Result<bool> {
    let watch = async {
        let mut printed = std::collections::HashMap::<String, String>::new();
        loop {
            let deploy = client.get_json(&context::path(&["deploys", id])).await?;
            for step in deploy["steps"].as_array().into_iter().flatten() {
                let name = ui::text(step, "name");
                let status = ui::text(step, "status");
                if printed.get(&name) != Some(&status) && status != "pending" {
                    let mark = match status.as_str() {
                        "succeeded" => console::style("✓").green().to_string(),
                        "failed" => console::style("✗").red().to_string(),
                        "skipped" => console::style("-").dim().to_string(),
                        _ => console::style("…").dim().to_string(),
                    };
                    let message = step["message"]
                        .as_str()
                        .map(|text| format!("  {}", ui::clean(text)))
                        .unwrap_or_default();
                    eprintln!("  {mark} {name}{message}");
                    printed.insert(name, status);
                }
            }
            match deploy["status"].as_str() {
                Some("live") => {
                    let url = deploy["url"]
                        .as_str()
                        .map(|url| format!(" at {}", ui::clean(url)))
                        .unwrap_or_default();
                    ui::done(&format!("Live{url}."));
                    return Ok::<bool, anyhow::Error>(true);
                }
                Some("failed") => {
                    let why = ui::text(&deploy, "failure.message");
                    eprintln!("{} The deploy failed: {why}", console::style("✗").red());
                    let missing = &deploy["failure"]["missing_secrets"];
                    if missing.as_array().is_some_and(|names| !names.is_empty()) {
                        ui::note(&format!(
                            "Set what's missing with hoist env set PROJECT {}",
                            ui::text(missing, "").replace(", ", " ")
                        ));
                    }
                    return Ok(false);
                }
                Some("rolled_back") => {
                    ui::warn("It was rolled back.");
                    return Ok(false);
                }
                _ => tokio::time::sleep(FOLLOW_POLL).await,
            }
        }
    };
    tokio::select! {
        result = watch => result,
        _ = tokio::signal::ctrl_c() => {
            ui::note(&format!("Stopped watching. The deploy keeps going; see it with hoist deploys show {id}."));
            Ok(true)
        }
    }
}

// hoist plan

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct PlanArgs {
    #[command(subcommand)]
    command: Option<PlanCommand>,
    /// The folder to plan. Default: this folder.
    folder: Option<PathBuf>,
    /// The project name the plan is for. Default: the folder's name.
    #[arg(long, short = 'p', value_name = "NAME")]
    project: Option<String>,
}

#[derive(Subcommand)]
enum PlanCommand {
    /// Show a saved plan.
    Show {
        /// Plan id.
        plan: String,
    },
    /// Change which account or region a planned resource uses.
    Set {
        /// Plan id.
        plan: String,
        /// The resource's name in the plan.
        resource: String,
        /// Account name or id to use.
        #[arg(long)]
        account: Option<String>,
        /// Region to use; "-" clears it.
        #[arg(long)]
        region: Option<String>,
    },
    /// Replace a plan with JSON from stdin (the shape hoist plan show --json prints).
    Edit {
        /// Plan id.
        plan: String,
    },
}

pub(crate) async fn plan(context: &Context, args: PlanArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    let print = |plan: Plan, accounts: Vec<Value>| {
        let value = serde_json::to_value(&plan)?;
        ui::show(&value, |_| {
            show_plan(&plan, &accounts);
            ui::note(&format!(
                "\nDeploy it with hoist deploy --project NAME --plan {}.",
                plan.id
            ));
        })
    };
    match args.command {
        None => {
            let (path, source) = folder_source(args.folder)?;
            let name = args
                .project
                .or_else(|| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .map(str::to_lowercase)
                })
                .context("Pass --project NAME")?;
            // A folder that is already a managed project's is planned within it.
            let environment = context.environment(&client).await?;
            let project = context::projects(&client)
                .await?
                .into_iter()
                .find(|project| {
                    project.environment_id == environment.id
                        && project.mode == Mode::Managed
                        && project.source == source
                });
            let plan: Plan = serde_json::from_value(
                client
                    .post_json(
                        "/plans",
                        &json!({
                            "source": source,
                            "name": name,
                            "pinned": null,
                            "project_id": project.map(|project| project.id),
                        }),
                    )
                    .await?,
            )?;
            print(plan, all_accounts(&client).await)
        }
        Some(PlanCommand::Show { plan }) => {
            let plan = plans::show(&client, &plan).await?;
            print(plan, all_accounts(&client).await)
        }
        Some(PlanCommand::Set {
            plan,
            resource,
            account,
            region,
        }) => {
            if account.is_none() && region.is_none() {
                return ui::usage("Pass --account, --region or both.");
            }
            if let Some(account) = account {
                let account = crate::accounts::account(&client, Some(&account), None, "").await?;
                plans::change(
                    &client,
                    &plan,
                    &resource,
                    "account",
                    &ui::text(&account, "id"),
                )
                .await?;
            }
            if let Some(region) = region {
                plans::change(&client, &plan, &resource, "region", &region).await?;
            }
            let plan = plans::show(&client, &plan).await?;
            print(plan, all_accounts(&client).await)
        }
        Some(PlanCommand::Edit { plan }) => {
            crate::environment::require_pipe()?;
            let edited = plans::read(std::io::stdin(), &plan)?;
            let saved = plans::save(&client, edited).await?;
            ui::show(&saved, |_| ui::done("Plan saved."))
        }
    }
}

// hoist deploys

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct DeploysArgs {
    #[command(subcommand)]
    command: Option<DeploysCommand>,
    /// Project name or id. Default: recent deploys of every project.
    project: Option<String>,
}

#[derive(Subcommand)]
enum DeploysCommand {
    /// Show one deploy: its steps, and with --log its build log.
    Show {
        /// Deploy id.
        deploy: String,
        /// Also print the build log the provider kept.
        #[arg(long)]
        log: bool,
    },
    /// Look up a deploy by the request key it was started with.
    Request {
        /// The request key (UUID).
        key: String,
        /// Who started it: an agent's id, when it wasn't you.
        #[arg(long, value_name = "ID")]
        actor: Option<String>,
    },
}

fn deploy_row(
    names: &std::collections::HashMap<String, String>,
) -> impl Fn(&Value) -> Vec<String> + '_ {
    move |item| {
        vec![
            item["project_id"]
                .as_str()
                .and_then(|id| names.get(id))
                .cloned()
                .unwrap_or_else(|| ui::text(item, "project_id")),
            ui::text(item, "status"),
            ui::when(item, "started_at"),
            item["duration_ms"]
                .as_i64()
                .map_or_else(|| "-".into(), |ms| format!("{}s", ms / 1000)),
            ui::text(item, "title"),
            ui::text(item, "started_by_name"),
            ui::text(item, "id"),
        ]
    }
}

pub(crate) async fn deploys(context: &Context, args: DeploysArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    match args.command {
        None => {
            let projects = context::projects(&client).await?;
            let names: std::collections::HashMap<String, String> = projects
                .iter()
                .map(|project| (project.id.clone(), project.name.clone()))
                .collect();
            let items = match &args.project {
                Some(wanted) => {
                    let project = context::project(context, &client, Some(wanted), "").await?;
                    client
                        .get_json(&context::path(&["projects", &project.id, "deploys"]))
                        .await?
                }
                None => client.get_json("/deploys").await?,
            };
            ui::list(
                &items,
                "No deploys yet.",
                deploy_row(&names),
                &[
                    "PROJECT", "STATUS", "STARTED", "TOOK", "MESSAGE", "BY", "ID",
                ],
            )
        }
        Some(DeploysCommand::Show { deploy, log }) => {
            context::check_id(&deploy, "a deploy")?;
            let found = client
                .get_json(&context::path(&["deploys", &deploy]))
                .await?;
            let mut lines = Vec::new();
            if log {
                lines = build_log(&client, &found).await.unwrap_or_default();
            }
            let output = if log {
                json!({"deploy": found, "log": lines})
            } else {
                found.clone()
            };
            ui::show(&output, |_| {
                ui::fields(&[
                    ("Deploy", ui::text(&found, "id")),
                    ("Status", ui::text(&found, "status")),
                    ("Started", ui::when(&found, "started_at")),
                    ("Finished", ui::when(&found, "finished_at")),
                    ("Message", ui::text(&found, "title")),
                    ("By", ui::text(&found, "started_by_name")),
                    ("Address", ui::text(&found, "url")),
                ]);
                if !found["failure"].is_null() {
                    ui::fields(&[("Failed", ui::text(&found, "failure.message"))]);
                }
                println!();
                for step in found["steps"].as_array().into_iter().flatten() {
                    println!(
                        "  {:<10} {}{}",
                        ui::text(step, "status"),
                        ui::text(step, "name"),
                        step["message"]
                            .as_str()
                            .map(|text| format!("  {}", ui::clean(text)))
                            .unwrap_or_default()
                    );
                }
                if log {
                    println!();
                    if lines.is_empty() {
                        ui::note("The provider kept no build log for this deploy.");
                    }
                    for line in &lines {
                        println!(
                            "{} {} {}",
                            ui::text(line, "timestamp"),
                            ui::text(line, "level"),
                            ui::text(line, "message")
                        );
                    }
                }
            })
        }
        Some(DeploysCommand::Request { key, actor }) => {
            let identity = hoista_server::application::ports::catalog::DeploymentAdmission::new(
                actor.as_deref().unwrap_or("person"),
                &key,
                &"0".repeat(64),
            )
            .or_else(|_| ui::usage("Give the request key (a UUID) the deploy started with."))?;
            let found = client
                .get_json(&format!(
                    "/deployment-requests/{}?{}",
                    identity.key(),
                    context::query(&[("actor", identity.actor())])
                ))
                .await
                .context("Couldn't look it up. The first attempt may still be running; check hoist deploys before starting another.")?;
            ui::show(&found, |found| {
                ui::fields(&[
                    ("Request", ui::text(found, "request_key")),
                    ("Deploy", ui::text(found, "deploy_id")),
                    ("Status", ui::text(found, "deployment.status")),
                ]);
            })
        }
    }
}

/// The provider's build log for each resource the deploy released.
async fn build_log(client: &LocalClient, deploy: &Value) -> anyhow::Result<Vec<Value>> {
    let mut lines = Vec::new();
    for release in deploy["releases"].as_array().into_iter().flatten() {
        let (Some(resource), Some(provider_deploy)) = (
            release["resource_id"].as_str(),
            release["release"]["reference"].as_str(),
        ) else {
            continue;
        };
        if let Ok(found) = client
            .get_json(&context::path(&[
                "resources",
                resource,
                "provider",
                "deployments",
                provider_deploy,
                "logs",
            ]))
            .await
        {
            lines.extend(found.as_array().cloned().unwrap_or_default());
        }
    }
    Ok(lines)
}

// hoist rollback / cancel

#[derive(Args)]
pub(crate) struct RollbackArgs {
    /// The deploy to put back. In a terminal, pick it from the project's deploys.
    deploy: Option<String>,
    /// The project, when picking the deploy from a list.
    #[arg(long, short = 'p')]
    project: Option<String>,
    /// A note saved with the rollback.
    #[arg(long, short = 'm', value_name = "TEXT")]
    message: Option<String>,
    /// A UUID that makes retrying safe.
    #[arg(long, value_name = "UUID")]
    request_key: Option<String>,
}

pub(crate) async fn rollback(context: &Context, args: RollbackArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    let deploy = match args.deploy {
        Some(deploy) => deploy,
        None => {
            let project = context::project(
                context,
                &client,
                args.project.as_deref(),
                "Name the deploy: hoist rollback DEPLOY. See hoist deploys PROJECT.",
            )
            .await?;
            let list = client
                .get_json(&context::path(&["projects", &project.id, "deploys"]))
                .await?;
            let earlier: Vec<Value> = list["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|deploy| deploy["status"] == "live" || deploy["status"] == "rolled_back")
                .cloned()
                .collect();
            if earlier.is_empty() {
                bail!(
                    "{} has no earlier deploy to put back.",
                    ui::clean(&project.name)
                );
            }
            if !ui::can_ask() {
                return ui::usage("Name the deploy: hoist rollback DEPLOY.");
            }
            let labels: Vec<String> = earlier
                .iter()
                .map(|deploy| {
                    format!(
                        "{}  {}  {}",
                        ui::when(deploy, "started_at"),
                        ui::text(deploy, "status"),
                        ui::text(deploy, "title")
                    )
                })
                .collect();
            ui::text(&earlier[ui::pick("Put back", &labels)?], "id")
        }
    };
    context::check_id(&deploy, "a deploy")?;
    let key = request_key(args.request_key)?;
    if !ui::confirm(&format!("Roll back to deploy {deploy}?"), true)? {
        return Ok(());
    }
    let started = client
        .post_json(
            &context::path(&["deploys", &deploy, "rollback"]),
            &json!({"request_key": key, "channel": "cli", "title": args.message}),
        )
        .await
        .with_context(|| {
            format!("The rollback didn't confirm. Retry safely with --request-key {key}.")
        })?;
    ui::show(&started, |started| {
        ui::done(&format!(
            "Rolling back. Follow it with hoist deploys show {}.",
            ui::text(started, "id")
        ));
    })
}

#[derive(Args)]
pub(crate) struct CancelArgs {
    /// Deploy id.
    deploy: String,
}

pub(crate) async fn cancel(context: &Context, args: CancelArgs) -> anyhow::Result<()> {
    context::check_id(&args.deploy, "a deploy")?;
    let client = context.client().await?;
    let cancelled = client
        .post_json(
            &context::path(&["deploys", &args.deploy, "cancel"]),
            &json!({}),
        )
        .await?;
    ui::show(&cancelled, |_| {
        ui::done(
            "Stopping at the next safe point. A provider step already under way may still finish.",
        );
    })
}

// hoist env

#[derive(Args)]
pub(crate) struct EnvArgs {
    #[command(subcommand)]
    command: EnvCommand,
}

#[derive(Subcommand)]
enum EnvCommand {
    /// List a project's variable names. Values are never shown.
    List {
        /// Project name or id.
        project: Option<String>,
    },
    /// Set variables. Values are typed hidden, piped, or read from a .env file; never in arguments.
    ///
    /// On a project Hoista manages, the new values go out in a new deploy,
    /// safe to retry with --request-key. On a watched project they're set at
    /// the provider, which redeploys it.
    Set(EnvSetArgs),
    /// Remove a variable.
    Remove {
        /// Project name or id.
        project: Option<String>,
        /// Variable name.
        name: Option<String>,
        /// The resource it belongs to, when more than one has it.
        #[arg(long, short = 'r', value_name = "RESOURCE")]
        resource: Option<String>,
    },
}

#[derive(Args)]
struct EnvSetArgs {
    /// Project name or id.
    project: Option<String>,
    /// Variable names. Each value is asked for hidden.
    names: Vec<String>,
    /// The resource the variables belong to, when the project has more than one.
    #[arg(long, short = 'r', value_name = "RESOURCE")]
    resource: Option<String>,
    /// Read one variable's value from stdin.
    #[arg(long, conflicts_with = "file")]
    value_stdin: bool,
    /// Read NAME=value lines from a .env file. With names, only those.
    #[arg(long, value_name = "PATH")]
    file: Option<PathBuf>,
    /// A note saved with the deploy.
    #[arg(long, short = 'm', value_name = "TEXT")]
    message: Option<String>,
    /// Deploy with a plan you reviewed, instead of planning again.
    #[arg(long, value_name = "PLAN")]
    plan: Option<String>,
    /// With --plan: only if the plan is still this revision.
    #[arg(long, value_name = "N", requires = "plan", value_parser = clap::value_parser!(i64).range(0..=9_007_199_254_740_991))]
    plan_revision: Option<i64>,
    /// A UUID that makes retrying safe. Retry with the same values.
    #[arg(long, value_name = "UUID")]
    request_key: Option<String>,
    /// Start the deploy and return without waiting.
    #[arg(long)]
    detach: bool,
    /// Watched projects: set them without the provider redeploying.
    #[arg(long)]
    no_redeploy: bool,
}

/// The resource variables go to: named, the only one that takes them, or picked.
async fn env_resource(
    client: &LocalClient,
    project: &Project,
    wanted: Option<&str>,
) -> anyhow::Result<Value> {
    let resources = context::resources(client, project).await?;
    if let Some(wanted) = wanted {
        return resources
            .iter()
            .find(|resource| resource["id"] == wanted || resource["name"] == wanted)
            .cloned()
            .with_context(|| {
                format!(
                    "{} has no resource called {}.",
                    ui::clean(&project.name),
                    ui::clean(wanted)
                )
            });
    }
    let takes: Vec<&Value> = resources
        .iter()
        .filter(|resource| {
            !matches!(
                resource["kind"].as_str(),
                Some("database" | "bucket" | "domain" | "volume")
            )
        })
        .collect();
    match takes.as_slice() {
        [one] => Ok((*one).clone()),
        [] => bail!(
            "{} has nothing that takes variables.",
            ui::clean(&project.name)
        ),
        _ if ui::can_ask() => {
            let names: Vec<String> = takes
                .iter()
                .map(|resource| {
                    format!(
                        "{} ({})",
                        ui::text(resource, "name"),
                        ui::text(resource, "provider")
                    )
                })
                .collect();
            Ok(takes[ui::pick("Resource", &names)?].clone())
        }
        _ => ui::usage("The project has more than one resource. Pass --resource NAME."),
    }
}

/// A request body holding values, cleared from memory when dropped.
struct Private(Value);
impl Drop for Private {
    fn drop(&mut self) {
        fn clear(value: &mut Value) {
            match value {
                Value::String(text) => text.zeroize(),
                Value::Array(items) => items.iter_mut().for_each(clear),
                Value::Object(map) => map.values_mut().for_each(clear),
                _ => {}
            }
        }
        clear(&mut self.0);
    }
}

/// The values to set, from a .env file, stdin, or hidden prompts.
fn values(args: &EnvSetArgs) -> anyhow::Result<Vec<(String, hoista_server::domain::Secret)>> {
    for name in &args.names {
        if !valid_env_name(name) {
            return ui::usage(format!(
                "{} isn't a variable name: use letters, digits and underscores, not starting with a digit.",
                ui::clean(name)
            ));
        }
    }
    if let Some(file) = &args.file {
        let path = std::path::absolute(file)?;
        let wanted = (!args.names.is_empty()).then_some(args.names.as_slice());
        return hoista_server::env_file::read_env_file(
            path.to_str().context("The file's path isn't UTF-8")?,
            wanted,
        )
        .map_err(|error| anyhow::anyhow!("{error}"));
    }
    if args.value_stdin {
        let [name] = args.names.as_slice() else {
            return ui::usage(
                "--value-stdin sets one variable: hoist env set PROJECT NAME --value-stdin.",
            );
        };
        return Ok(vec![(name.clone(), crate::environment::value(true, name)?)]);
    }
    if !ui::can_ask() {
        return ui::usage(
            "Pipe the value with --value-stdin, or use --file. Values never go in arguments.",
        );
    }
    let names = if args.names.is_empty() {
        let name = ui::ask("Variable name", None)?;
        if !valid_env_name(&name) {
            return ui::usage(format!("{} isn't a variable name.", ui::clean(&name)));
        }
        vec![name]
    } else {
        args.names.clone()
    };
    names
        .into_iter()
        .map(|name| {
            let value = crate::environment::value(false, &format!("{name} (hidden)"))?;
            Ok((name, value))
        })
        .collect()
}

pub(crate) async fn env(context: &Context, args: EnvArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    match args.command {
        EnvCommand::List { project } => {
            let project = context::project(
                context,
                &client,
                project.as_deref(),
                "Name the project: hoist env list PROJECT.",
            )
            .await?;
            let resources = context::resources(&client, &project).await?;
            let resource_name = |id: &Value| {
                resources
                    .iter()
                    .find(|resource| resource["id"] == *id)
                    .map_or_else(|| ui::text(id, ""), |resource| ui::text(resource, "name"))
            };
            let items: Vec<Value> = if project.mode == Mode::Managed {
                let page = client
                    .get_json(&context::path(&["projects", &project.id, "secrets"]))
                    .await?;
                page["items"].as_array().cloned().unwrap_or_default()
            } else {
                let mut items = Vec::new();
                for resource in &resources {
                    let id = ui::text(resource, "id");
                    let Ok(details) = client
                        .get_json(&context::path(&["resources", &id, "provider"]))
                        .await
                    else {
                        continue;
                    };
                    for name in details["variable_names"].as_array().into_iter().flatten() {
                        items.push(json!({
                            "name": name,
                            "resource_id": id,
                            "set_at": null,
                            "set_by": null,
                            "browser_visible": null,
                        }));
                    }
                    for name in details["shared_variable_names"]
                        .as_array()
                        .into_iter()
                        .flatten()
                    {
                        items.push(json!({
                            "name": name,
                            "resource_id": id,
                            "shared": true,
                            "set_at": null,
                            "set_by": null,
                            "browser_visible": null,
                        }));
                    }
                }
                items
            };
            ui::list(
                &Value::Array(items),
                &format!("{} has no variables.", ui::clean(&project.name)),
                |item| {
                    vec![
                        ui::text(item, "name"),
                        if item["shared"] == true {
                            "shared".into()
                        } else {
                            resource_name(&item["resource_id"])
                        },
                        ui::when(item, "set_at"),
                        ui::text(item, "set_by"),
                        ui::text(item, "browser_visible"),
                    ]
                },
                &["NAME", "RESOURCE", "SET", "BY", "IN BROWSER CODE"],
            )
        }
        EnvCommand::Set(args) => {
            let project = context::project(
                context,
                &client,
                args.project.as_deref(),
                "Name the project: hoist env set PROJECT NAME.",
            )
            .await?;
            let values = values(&args)?;
            if values.is_empty() {
                return ui::usage("Nothing to set.");
            }
            if project.mode == Mode::Managed {
                return deploy_values(&client, &project, &args, values).await;
            }
            let target = env_resource(&client, &project, args.resource.as_deref()).await?;
            let resource_id = ui::text(&target, "id");
            let names: Vec<String> = values.iter().map(|(name, _)| name.clone()).collect();
            let body = Private(json!({
                "variables": values.iter().map(|(name, value)| (name.clone(), json!(value.expose()))).collect::<serde_json::Map<_, _>>(),
                "shared": false,
                "redeploy": !args.no_redeploy,
            }));
            client
                .put_json(
                    &context::path(&["resources", &resource_id, "provider", "variables"]),
                    &body.0,
                )
                .await?;
            ui::show(&json!({"set": names, "resource_id": resource_id}), |_| {
                ui::done(&format!(
                    "Set {} on {}{}.",
                    names.join(", "),
                    ui::text(&target, "name"),
                    if args.no_redeploy {
                        ""
                    } else {
                        "; the provider redeploys it"
                    }
                ));
            })
        }
        EnvCommand::Remove {
            project,
            name,
            resource,
        } => {
            let project = context::project(
                context,
                &client,
                project.as_deref(),
                "Name the project: hoist env remove PROJECT NAME.",
            )
            .await?;
            let name = ui::need(
                name,
                "Variable name",
                "Name the variable: hoist env remove PROJECT NAME.",
            )?;
            if !valid_env_name(&name) {
                return ui::usage(format!("{} isn't a variable name.", ui::clean(&name)));
            }
            let target = env_resource(&client, &project, resource.as_deref()).await?;
            let resource_id = ui::text(&target, "id");
            if !ui::confirm(
                &format!("Remove {name} from {}?", ui::text(&target, "name")),
                false,
            )? {
                return Ok(());
            }
            let path = if project.mode == Mode::Managed {
                context::path(&[
                    "projects",
                    &project.id,
                    "resources",
                    &resource_id,
                    "secrets",
                    &name,
                ])
            } else {
                context::path(&["resources", &resource_id, "provider", "variables", &name])
            };
            let removed = client.delete_json(&path).await?;
            ui::show(&removed, |_| ui::done(&format!("Removed {name}.")))
        }
    }
}

/// A managed project's new values go out with a deploy, so the release
/// and its values change together.
async fn deploy_values(
    client: &LocalClient,
    project: &Project,
    args: &EnvSetArgs,
    values: Vec<(String, hoista_server::domain::Secret)>,
) -> anyhow::Result<()> {
    if values.len() > MAX_VALUES
        || values
            .iter()
            .map(|(_, value)| value.expose().len())
            .sum::<usize>()
            > MAX_VALUES_BYTES
    {
        return ui::usage("Set at most 64 variables and 64 KiB at once.");
    }
    let plan = match &args.plan {
        Some(plan) => plans::show(client, plan).await?,
        None => {
            if matches!(project.source, Source::Unlinked) {
                bail!(
                    "{} has no source to deploy from. Link one with hoist link {}.",
                    ui::clean(&project.name),
                    ui::clean(&project.name)
                );
            }
            serde_json::from_value(
                client
                    .post_json(
                        "/plans",
                        &json!({
                            "source": project.source,
                            "name": project.name,
                            "pinned": null,
                            "project_id": project.id,
                        }),
                    )
                    .await?,
            )
            .context("The plan didn't read")?
        }
    };
    let revision = args.plan_revision.unwrap_or(plan.revision);
    let key = request_key(args.request_key.clone())?;
    let body = Private(json!({
        "request_key": key,
        "plan_id": plan.id,
        "expected_plan_revision": revision,
        "changes": values.iter().map(|(name, value)| json!({"name": name, "value": value.expose()})).collect::<Vec<_>>(),
        "title": args.message,
        "channel": "cli",
    }));
    let started = client
        .post_json(
            &context::path(&["projects", &project.id, "environment", "deploy"]),
            &body.0,
        )
        .await
        .with_context(|| {
            format!(
                "The deploy didn't start, or its answer was lost. Retry with the same values and: --plan {} --plan-revision {revision} --request-key {key}",
                plan.id
            )
        })?;
    let id = ui::text(&started, "id");
    let mut started = started;
    started["request_key"] = json!(key);
    started["plan_revision"] = json!(revision);
    if args.detach || ui::mode().json {
        return ui::show(&started, |_| {
            ui::done(&format!(
                "Deploying the new values (deploy {id}, request key {key})."
            ));
        });
    }
    ui::note(&format!(
        "Deploying the new values to {} (deploy {id})",
        ui::clean(&project.name)
    ));
    if !follow(client, &id).await? {
        bail!(
            "The values weren't released. Retry safely with --request-key {key} and the same values."
        );
    }
    Ok(())
}
