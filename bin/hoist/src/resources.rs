//! A project's resources (services, databases, buckets), its databases, and
//! its custom domains.

use crate::{
    context::{self, Context},
    projects::{self, DeleteArgs},
    ui,
};
use anyhow::Context as _;
use clap::{Args, Subcommand};
use hoista_server::client::LocalClient;
use serde_json::{Value, json};

/// Which resource: by id, or by name with its project.
#[derive(Args, Clone)]
pub(crate) struct Target {
    /// Resource name or id. In a terminal, pick it from a list instead.
    resource: Option<String>,
    /// The resource's project, when naming it by name.
    #[arg(long, short = 'p', value_name = "PROJECT")]
    project: Option<String>,
}

impl Target {
    async fn find(&self, context: &Context, client: &LocalClient) -> anyhow::Result<Value> {
        context::resource(
            context,
            client,
            self.resource.as_deref(),
            self.project.as_deref(),
        )
        .await
    }
}

// hoist resources

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct ResourcesArgs {
    #[command(subcommand)]
    command: Option<ResourcesCommand>,
    /// Project name or id. Default: the project linked to this folder.
    project: Option<String>,
}

#[derive(Subcommand)]
enum ResourcesCommand {
    /// Show a resource: its provider, account, address and what it connects to.
    Show(Target),
    /// Restart what's running, without building again.
    Restart(Target),
    /// Build and start the current version again, at the provider.
    Redeploy(Target),
    /// Stop what's running at the provider.
    Stop(Target),
    /// Pause a resource (scale to zero or suspend, as the provider does it).
    Pause(Target),
    /// Start a paused resource again.
    Resume(Target),
    /// Move resources to another project in Hoista. Nothing changes at the provider.
    Move {
        /// Resource ids.
        #[arg(required = true)]
        resources: Vec<String>,
        /// The project to move them to.
        #[arg(
            long,
            value_name = "PROJECT",
            conflicts_with = "new",
            required_unless_present = "new"
        )]
        to: Option<String>,
        /// Make a new project for one resource, with this name.
        #[arg(long, value_name = "NAME")]
        new: Option<String>,
    },
    /// Run one of the provider's own operations on a resource, such as a backup or a scale change.
    Run {
        #[command(flatten)]
        target: Target,
        /// The operation's name. Leave it out to list them.
        #[arg(long, short = 'o', value_name = "NAME")]
        operation: Option<String>,
        /// An argument, as NAME=VALUE. VALUE is read as JSON when it is valid JSON.
        #[arg(long = "arg", value_name = "NAME=VALUE")]
        args: Vec<String>,
    },
    /// Stop watching a resource. It keeps running at its provider.
    Remove(Target),
    /// Delete a resource at its provider, after saving a copy of its data.
    Delete {
        #[command(flatten)]
        target: Target,
        /// Show what would be deleted, and delete nothing.
        #[arg(long)]
        dry_run: bool,
        /// The name, typed to confirm, for scripts.
        #[arg(long, value_name = "NAME")]
        confirm: Option<String>,
        /// Delete only if the --dry-run fingerprint still matches.
        #[arg(long, value_name = "HASH")]
        fingerprint: Option<String>,
    },
}

fn args_json(pairs: &[String]) -> anyhow::Result<Value> {
    let mut args = serde_json::Map::new();
    for pair in pairs {
        let Some((name, value)) = pair.split_once('=') else {
            return ui::usage(format!(
                "Write --arg as NAME=VALUE, not {}.",
                ui::clean(pair)
            ));
        };
        let value = serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_owned()));
        args.insert(name.to_owned(), value);
    }
    Ok(Value::Object(args))
}

pub(crate) async fn resources(context: &Context, args: ResourcesArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    let Some(command) = args.command else {
        let project = context::project(
            context,
            &client,
            args.project.as_deref(),
            "Name the project: hoist resources PROJECT.",
        )
        .await?;
        let items = context::resources(&client, &project).await?;
        return ui::list(
            &Value::Array(items),
            &format!("{} has no resources.", ui::clean(&project.name)),
            |item| {
                vec![
                    ui::text(item, "name"),
                    ui::text(item, "kind"),
                    ui::text(item, "provider"),
                    ui::text(item, "status"),
                    ui::text(item, "region"),
                    ui::text(item, "url"),
                    ui::text(item, "id"),
                ]
            },
            &[
                "NAME", "KIND", "PROVIDER", "STATUS", "REGION", "ADDRESS", "ID",
            ],
        );
    };
    match command {
        ResourcesCommand::Show(target) => {
            let resource = target.find(context, &client).await?;
            let id = ui::text(&resource, "id");
            let detail = client
                .get_json(&context::path(&["resources", &id, "details"]))
                .await?;
            ui::show(&detail, |detail| {
                ui::fields(&[
                    ("Resource", ui::text(detail, "resource.name")),
                    ("Kind", ui::text(detail, "resource.kind")),
                    ("Provider", ui::text(detail, "resource.provider")),
                    ("Account", ui::text(detail, "account.label")),
                    ("At the provider", ui::text(detail, "resource.provider_ref")),
                    ("Region", ui::text(detail, "resource.region")),
                    ("Status", ui::text(detail, "resource.status")),
                    ("Mode", ui::text(detail, "resource.mode")),
                    ("Address", ui::text(detail, "resource.url")),
                    ("Project", ui::text(detail, "project.name")),
                    ("Id", id.clone()),
                ]);
                for (title, key) in [("Uses", "dependencies"), ("Used by", "used_by")] {
                    let items = detail[key].as_array().cloned().unwrap_or_default();
                    if !items.is_empty() {
                        println!("\n{title}:");
                        for item in items {
                            println!(
                                "  {}  via {}",
                                ui::text(&item, "resource.name"),
                                ui::text(&item, "via")
                            );
                        }
                    }
                }
            })
        }
        ResourcesCommand::Restart(target) => {
            provider_action(context, &client, &target, "restart", "Restarting").await
        }
        ResourcesCommand::Redeploy(target) => {
            provider_action(context, &client, &target, "redeploy", "Redeploying").await
        }
        ResourcesCommand::Stop(target) => {
            let resource = target.find(context, &client).await?;
            if !ui::confirm(
                &format!("Stop {} at the provider?", ui::text(&resource, "name")),
                false,
            )? {
                return Ok(());
            }
            provider_action(
                context,
                &client,
                &Target {
                    resource: Some(ui::text(&resource, "id")),
                    project: None,
                },
                "stop",
                "Stopping",
            )
            .await
        }
        ResourcesCommand::Pause(target) => pause(context, &client, &target, "pause").await,
        ResourcesCommand::Resume(target) => pause(context, &client, &target, "resume").await,
        ResourcesCommand::Move { resources, to, new } => {
            for id in &resources {
                context::check_id(id, "a resource")?;
            }
            let moved = if let Some(name) = new {
                let [one] = resources.as_slice() else {
                    return ui::usage("--new makes a project for one resource. Name just one.");
                };
                client
                    .post_json(
                        &context::path(&["resources", one, "split"]),
                        &json!({"name": name}),
                    )
                    .await?
            } else {
                let project = context::project(context, &client, to.as_deref(), "").await?;
                client
                    .post_json(
                        &context::path(&["projects", &project.id, "resources", "move"]),
                        &json!({"resource_ids": resources}),
                    )
                    .await?
            };
            ui::show(&moved, |_| ui::done("Moved."))
        }
        ResourcesCommand::Run {
            target,
            operation,
            args,
        } => {
            let resource = target.find(context, &client).await?;
            let id = ui::text(&resource, "id");
            let operations = client
                .get_json(&context::path(&[
                    "resources",
                    &id,
                    "provider",
                    "operations",
                ]))
                .await?;
            let list = operations.as_array().cloned().unwrap_or_default();
            let operation = match operation {
                Some(operation) => operation,
                None if ui::can_ask() && !list.is_empty() => {
                    let labels: Vec<String> = list
                        .iter()
                        .map(|item| {
                            format!(
                                "{}  {}",
                                ui::text(item, "name"),
                                ui::text(item, "description")
                            )
                        })
                        .collect();
                    ui::text(&list[ui::pick("Operation", &labels)?], "name")
                }
                None => {
                    return ui::list(
                        &operations,
                        "The provider has no operations for this resource.",
                        |item| {
                            vec![
                                ui::text(item, "name"),
                                ui::text(item, "risk"),
                                ui::text(item, "description"),
                            ]
                        },
                        &["OPERATION", "RISK", "WHAT IT DOES"],
                    );
                }
            };
            let info = list
                .iter()
                .find(|item| item["name"] == operation.as_str())
                .cloned();
            let mut body = args_json(&args)?;
            if let Some(info) = &info {
                for arg in info["args"].as_array().into_iter().flatten() {
                    let name = ui::text(arg, "name");
                    if arg["required"] == true && body.get(&name).is_none() {
                        let value = ui::need(
                            None,
                            &name,
                            &format!("{operation} needs --arg {name}=VALUE."),
                        )?;
                        body[&name] = serde_json::from_str(&value).unwrap_or(Value::String(value));
                    }
                }
                if info["risk"] != "read"
                    && !ui::confirm(
                        &format!("Run {operation} on {}?", ui::text(&resource, "name")),
                        false,
                    )?
                {
                    return Ok(());
                }
            }
            let ran = client
                .post_json(
                    &context::path(&["resources", &id, "provider", "operations", &operation]),
                    &json!({"args": body}),
                )
                .await?;
            ui::show(&ran, |ran| {
                println!("{}", serde_json::to_string_pretty(ran).unwrap_or_default());
            })
        }
        ResourcesCommand::Remove(target) => {
            let resource = target.find(context, &client).await?;
            if !ui::confirm(
                &format!(
                    "Stop watching {}? It keeps running at its provider.",
                    ui::text(&resource, "name")
                ),
                false,
            )? {
                return Ok(());
            }
            let id = ui::text(&resource, "id");
            client
                .post_json(
                    &context::path(&["resources", &id, "stop-watching"]),
                    &json!({}),
                )
                .await?;
            ui::show(&json!({"removed": true, "resource_id": id}), |_| {
                ui::done(&format!(
                    "Stopped watching {}.",
                    ui::text(&resource, "name")
                ));
            })
        }
        ResourcesCommand::Delete {
            target,
            dry_run,
            confirm,
            fingerprint,
        } => {
            let resource = target.find(context, &client).await?;
            let args = DeleteArgs {
                target: None,
                dry_run,
                confirm,
                fingerprint,
                skip_backup: false,
            };
            projects::delete(
                &client,
                "resources",
                &ui::text(&resource, "id"),
                &ui::text(&resource, "name"),
                &args,
            )
            .await
        }
    }
}

async fn provider_action(
    context: &Context,
    client: &LocalClient,
    target: &Target,
    action: &str,
    doing: &str,
) -> anyhow::Result<()> {
    let resource = target.find(context, client).await?;
    let id = ui::text(&resource, "id");
    let result = client
        .post_json(
            &context::path(&["resources", &id, "provider", "actions"]),
            &json!({"action": action}),
        )
        .await?;
    ui::show(&result, |result| {
        let summary = result["summary"]
            .as_str()
            .map(ui::clean)
            .unwrap_or_else(|| format!("{doing} {}.", ui::text(&resource, "name")));
        ui::done(&summary);
    })
}

async fn pause(
    context: &Context,
    client: &LocalClient,
    target: &Target,
    action: &str,
) -> anyhow::Result<()> {
    let resource = target.find(context, client).await?;
    let id = ui::text(&resource, "id");
    let result = client
        .post_json(&context::path(&["resources", &id, action]), &json!({}))
        .await
        .context("The provider didn't confirm. Check the resource's status before trying again.")?;
    ui::show(&result, |_| {
        ui::done(&format!(
            "{} {}.",
            if action == "pause" {
                "Paused"
            } else {
                "Resumed"
            },
            ui::text(&resource, "name")
        ));
    })
}

// hoist db

#[derive(Args)]
pub(crate) struct DbArgs {
    #[command(subcommand)]
    command: DbCommand,
}

#[derive(Subcommand)]
enum DbCommand {
    /// What the provider reports about a database: branches, computes, size.
    Show(Target),
    /// Save a copy of a database to this computer.
    Export(Target),
    /// List the database copies saved on this computer.
    Exports,
    /// Recent operations the provider ran on a database.
    Operations(Target),
}

pub(crate) async fn db(context: &Context, args: DbArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    match args.command {
        DbCommand::Show(target) => {
            let resource = target.find(context, &client).await?;
            let found = client
                .get_json(&context::path(&[
                    "resources",
                    &ui::text(&resource, "id"),
                    "database",
                ]))
                .await?;
            ui::show(&found, |found| {
                ui::fields(&[
                    ("Database", ui::text(&resource, "name")),
                    ("Observed", ui::when(found, "observed_at")),
                ]);
                let mut table = ui::Table::new(&["BRANCH", "SIZE", "ID"]);
                for branch in found["branches"].as_array().into_iter().flatten() {
                    table.row(vec![
                        ui::text(branch, "name"),
                        branch["logical_size_bytes"]
                            .as_u64()
                            .map_or_else(|| "-".into(), bytes),
                        ui::text(branch, "id"),
                    ]);
                }
                if !table.is_empty() {
                    println!();
                    table.print();
                }
            })
        }
        DbCommand::Export(target) => {
            let resource = target.find(context, &client).await?;
            ui::note(&format!(
                "Saving a copy of {}. Large databases take a while.",
                ui::text(&resource, "name")
            ));
            let saved = client
                .post_json_long_running(
                    &context::path(&["resources", &ui::text(&resource, "id"), "export"]),
                    &json!({}),
                )
                .await
                .context(
                    "The export didn't confirm. Check hoist db exports before trying again.",
                )?;
            ui::show(&saved, |saved| {
                let paths: Vec<String> = saved
                    .as_array()
                    .or_else(|| saved["items"].as_array())
                    .into_iter()
                    .flatten()
                    .map(|item| ui::text(item, "path"))
                    .collect();
                ui::done(&format!(
                    "Saved: {}",
                    if paths.is_empty() {
                        ui::text(saved, "path")
                    } else {
                        paths.join(", ")
                    }
                ));
            })
        }
        DbCommand::Exports => {
            let mut items = Vec::new();
            let mut path = "/database-exports".to_owned();
            for _ in 0..100 {
                let page = client.get_json(&path).await?;
                items.extend(page["items"].as_array().cloned().unwrap_or_default());
                let Some(cursor) = page["next_cursor"].as_str() else {
                    break;
                };
                path = format!(
                    "/database-exports?{}",
                    context::query(&[("cursor", cursor)])
                );
            }
            ui::list(
                &Value::Array(items),
                "No database copies on this computer.",
                |item| {
                    vec![
                        ui::text(item, "database_name"),
                        ui::when(item, "completed_at"),
                        item["bytes"].as_u64().map_or_else(|| "-".into(), bytes),
                        ui::text(item, "path"),
                    ]
                },
                &["DATABASE", "SAVED", "SIZE", "FILE"],
            )
        }
        DbCommand::Operations(target) => {
            let resource = target.find(context, &client).await?;
            let found = client
                .get_json(&context::path(&[
                    "resources",
                    &ui::text(&resource, "id"),
                    "database",
                    "operations",
                ]))
                .await?;
            ui::list(
                &found,
                "No operations reported.",
                |item| {
                    vec![
                        ui::text(item, "action"),
                        ui::text(item, "status"),
                        ui::when(item, "created_at"),
                        ui::text(item, "id"),
                    ]
                },
                &["ACTION", "STATUS", "STARTED", "ID"],
            )
        }
    }
}

/// `1.4 GB` from a byte count.
fn bytes(count: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = count as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < units.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{count} B")
    } else {
        format!("{value:.1} {}", units[unit])
    }
}

// hoist domains

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct DomainsArgs {
    #[command(subcommand)]
    command: Option<DomainsCommand>,
    /// Project name or id. Default: the project linked to this folder.
    project: Option<String>,
}

#[derive(Subcommand)]
enum DomainsCommand {
    /// Add a custom domain to a project's resource.
    Add {
        /// The domain, such as api.example.com.
        hostname: String,
        /// Project name or id.
        #[arg(long, short = 'p')]
        project: Option<String>,
        /// The resource that answers on it, when the project has more than one.
        #[arg(long, short = 'r')]
        resource: Option<String>,
        /// The DNS zone it's in. Default: its last two labels, such as example.com.
        #[arg(long)]
        zone: Option<String>,
        /// Create the zone at the provider if it isn't there.
        #[arg(long)]
        create_zone: bool,
    },
    /// Look up a domain's DNS and certificate again now.
    Check {
        /// Domain name or id.
        domain: String,
        /// Project name or id, when naming the domain by name.
        #[arg(long, short = 'p')]
        project: Option<String>,
    },
    /// Remove a custom domain.
    Remove {
        /// Domain name or id.
        domain: String,
        /// Project name or id, when naming the domain by name.
        #[arg(long, short = 'p')]
        project: Option<String>,
    },
}

async fn project_domains(client: &LocalClient, project_id: &str) -> anyhow::Result<Vec<Value>> {
    let page = client
        .get_json(&context::path(&["projects", project_id, "domains"]))
        .await?;
    Ok(page["items"].as_array().cloned().unwrap_or_default())
}

async fn domain_id(
    context: &Context,
    client: &LocalClient,
    wanted: &str,
    project: Option<&str>,
) -> anyhow::Result<String> {
    if !wanted.contains('.') {
        context::check_id(wanted, "a domain")?;
        return Ok(wanted.to_owned());
    }
    let project = context::project(
        context,
        client,
        project,
        "Name the project: --project PROJECT.",
    )
    .await?;
    project_domains(client, &project.id)
        .await?
        .iter()
        .find(|domain| domain["hostname"] == wanted)
        .map(|domain| ui::text(domain, "id"))
        .with_context(|| {
            format!(
                "{} has no domain {}.",
                ui::clean(&project.name),
                ui::clean(wanted)
            )
        })
}

pub(crate) async fn domains(context: &Context, args: DomainsArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    match args.command {
        None => {
            let project = context::project(
                context,
                &client,
                args.project.as_deref(),
                "Name the project: hoist domains PROJECT.",
            )
            .await?;
            let items = project_domains(&client, &project.id).await?;
            ui::list(
                &Value::Array(items),
                &format!(
                    "{} has no custom domains. Add one with hoist domains add HOSTNAME.",
                    ui::clean(&project.name)
                ),
                |item| {
                    vec![
                        ui::text(item, "hostname"),
                        ui::text(item, "observed.dns"),
                        ui::text(item, "observed.https"),
                        ui::when(item, "checked_at"),
                        ui::text(item, "id"),
                    ]
                },
                &["DOMAIN", "DNS", "HTTPS", "CHECKED", "ID"],
            )
        }
        Some(DomainsCommand::Add {
            hostname,
            project,
            resource,
            zone,
            create_zone,
        }) => {
            let project = context::project(
                context,
                &client,
                project.as_deref(),
                "Name the project: --project PROJECT.",
            )
            .await?;
            let target =
                context::resource(context, &client, resource.as_deref(), Some(&project.id)).await?;
            let zone = zone.unwrap_or_else(|| {
                let labels: Vec<&str> = hostname.trim_end_matches('.').split('.').collect();
                labels[labels.len().saturating_sub(2)..].join(".")
            });
            let added = client
                .post_json(
                    &context::path(&["projects", &project.id, "domains"]),
                    &json!({"resource_id": target["id"], "hostname": hostname, "zone_name": zone, "create_zone": create_zone}),
                )
                .await
                .context("The domain didn't confirm. Check hoist domains and the provider's DNS before trying again.")?;
            ui::show(&added, |added| {
                ui::done(&format!("Added {}. Waiting for DNS.", ui::clean(&hostname)));
                let records = added["observed"]["dns_records"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                if !records.is_empty() {
                    println!("\nAdd one of these at your DNS provider:");
                    for record in records {
                        println!(
                            "  {}  {}  {}",
                            ui::text(&record, "record_type"),
                            ui::text(&record, "name"),
                            ui::text(&record, "value")
                        );
                    }
                }
                for name in added["observed"]["nameservers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    println!("  nameserver {}", ui::text(name, ""));
                }
            })
        }
        Some(DomainsCommand::Check { domain, project }) => {
            let id = domain_id(context, &client, &domain, project.as_deref()).await?;
            let checked = client
                .post_json(&context::path(&["domains", &id, "check"]), &json!({}))
                .await?;
            ui::show(&checked, |checked| {
                ui::fields(&[
                    ("Domain", ui::text(checked, "hostname")),
                    ("DNS", ui::text(checked, "observed.dns")),
                    ("HTTPS", ui::text(checked, "observed.https")),
                ]);
                for warning in checked["observed"]["warnings"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    ui::warn(&ui::text(warning, ""));
                }
            })
        }
        Some(DomainsCommand::Remove { domain, project }) => {
            let id = domain_id(context, &client, &domain, project.as_deref()).await?;
            if !ui::confirm(&format!("Remove {}?", ui::clean(&domain)), false)? {
                return Ok(());
            }
            let removed = client
                .delete_json(&context::path(&["domains", &id]))
                .await
                .context("The removal didn't confirm. Check the domain at the provider before trying again.")?;
            ui::show(&removed, |_| {
                ui::done(&format!("Removed {}.", ui::clean(&domain)))
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_arguments_read_json_when_they_can() {
        let args = args_json(&[
            "count=2".into(),
            "name=web".into(),
            "on=true".into(),
            "list=[1]".into(),
        ])
        .unwrap();
        assert_eq!(args["count"], 2);
        assert_eq!(args["name"], "web");
        assert_eq!(args["on"], true);
        assert_eq!(args["list"][0], 1);
        assert!(args_json(&["broken".into()]).is_err());
    }

    #[test]
    fn sizes_read_like_people_write_them() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(1_400_000_000), "1.4 GB");
    }
}
