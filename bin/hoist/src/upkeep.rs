//! Safety and upkeep: backups, operations, webhooks, settings and profile,
//! and the engine on this computer (start, doctor, diagnostics, daemon,
//! service, tools).

use crate::{
    context::{self, Context},
    ui,
};
use anyhow::Context as _;
use clap::{Args, Subcommand};
use hoista_server::{
    application::StateStore,
    domain::Profile,
    infrastructure::{
        paths::StatePaths,
        store::{SqliteStore, StoreError},
    },
    {
        client::LocalClient,
        contracts::{self, ProfileDto},
    },
};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

// hoist backups

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct BackupsArgs {
    #[command(subcommand)]
    command: Option<BackupsCommand>,
}

#[derive(Subcommand)]
enum BackupsCommand {
    /// Save a backup now.
    Now,
    /// Restore a backup into a fresh Hoista.
    Restore {
        /// The backup's name from hoist backups.
        name: String,
    },
    /// Save everything, sign-ins included, in one encrypted file to move to another computer.
    Export {
        /// The file to write. It is never overwritten.
        file: PathBuf,
        /// Read the passphrase from stdin.
        #[arg(long)]
        passphrase_stdin: bool,
    },
    /// Bring in a file from hoist backups export.
    Import {
        /// The file.
        file: PathBuf,
        /// Only check it opens and show what's in it.
        #[arg(long)]
        check: bool,
        /// Read the passphrase from stdin.
        #[arg(long)]
        passphrase_stdin: bool,
    },
    /// Imports that stopped partway, after a crash.
    Recovery,
    /// Give up an import that stopped partway.
    Abandon {
        /// Its id from hoist backups recovery.
        id: String,
    },
}

pub(crate) async fn backups(context: &Context, args: BackupsArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    match args.command {
        None => {
            let found = client.get_json("/backups").await?;
            ui::list(
                &found,
                "No backups yet. Save one with hoist backups now.",
                |item| {
                    vec![
                        ui::text(item, "name"),
                        ui::when(item, "created_at"),
                        ui::text(item, "projects"),
                        ui::text(item, "accounts"),
                    ]
                },
                &["NAME", "SAVED", "PROJECTS", "ACCOUNTS"],
            )
        }
        Some(BackupsCommand::Now) => {
            let saved = client.post_json("/backups", &json!({})).await?;
            ui::show(&saved, |saved| {
                ui::done(&format!("Saved {}.", ui::text(saved, "name")))
            })
        }
        Some(BackupsCommand::Restore { name }) => {
            if !name.starts_with("backup-") || name.contains(['/', '\\']) {
                return ui::usage("Use a backup's name from hoist backups.");
            }
            if !ui::confirm(
                &format!("Restore {name}? This only works on a Hoista with nothing in it."),
                false,
            )? {
                return Ok(());
            }
            let restored = client
                .post_json(&format!("/backups/{name}/restore"), &json!({}))
                .await?;
            ui::show(&restored, |_| {
                ui::done(
                    "Restored. Projects start as watching; manage them again with hoist projects manage.",
                );
            })
        }
        Some(BackupsCommand::Export {
            file,
            passphrase_stdin,
        }) => {
            crate::transfer::run(&client, "export", &file.to_string_lossy(), passphrase_stdin).await
        }
        Some(BackupsCommand::Import {
            file,
            check,
            passphrase_stdin,
        }) => {
            let operation = if check { "inspect" } else { "restore" };
            crate::transfer::run(
                &client,
                operation,
                &file.to_string_lossy(),
                passphrase_stdin,
            )
            .await
        }
        Some(BackupsCommand::Recovery) => {
            let found = client.get_json("/transfer/recovery").await?;
            ui::show(&found, |found| {
                println!(
                    "{}",
                    serde_json::to_string_pretty(found).unwrap_or_default()
                )
            })
        }
        Some(BackupsCommand::Abandon { id }) => {
            uuid::Uuid::parse_str(&id)
                .or_else(|_| ui::usage("Use an id from hoist backups recovery."))?;
            let abandoned = client
                .post_json(
                    &context::path(&["transfer", "recovery", &id, "abandon"]),
                    &json!({}),
                )
                .await?;
            ui::show(&abandoned, |_| ui::done("Abandoned."))
        }
    }
}

// hoist operations

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct OperationsArgs {
    #[command(subcommand)]
    command: Option<OperationsCommand>,
}

#[derive(Subcommand)]
enum OperationsCommand {
    /// Say whether an operation whose result was lost happened at the provider.
    Check {
        /// Operation id from hoist operations.
        id: String,
        /// It happened.
        #[arg(
            long,
            conflicts_with = "did_not_happen",
            required_unless_present = "did_not_happen"
        )]
        happened: bool,
        /// It didn't happen.
        #[arg(long)]
        did_not_happen: bool,
    },
}

pub(crate) async fn operations(context: &Context, args: OperationsArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    match args.command {
        None => {
            let found = client.get_json("/operations").await?;
            ui::list(
                &found,
                "No provider operations recorded.",
                |item| {
                    vec![
                        ui::text(item, "kind"),
                        ui::text(item, "target"),
                        ui::text(item, "provider"),
                        ui::text(item, "state"),
                        ui::when(item, "started_at"),
                        ui::text(item, "id"),
                    ]
                },
                &["OPERATION", "TARGET", "PROVIDER", "STATE", "STARTED", "ID"],
            )
        }
        Some(OperationsCommand::Check { id, happened, .. }) => {
            uuid::Uuid::parse_str(&id)
                .or_else(|_| ui::usage("Use an id from hoist operations."))?;
            let checked = client
                .post_json(
                    &context::path(&["operations", &id, "check"]),
                    &json!({"happened": happened}),
                )
                .await?;
            ui::show(&checked, |_| ui::done("Recorded."))
        }
    }
}

// hoist webhooks

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct WebhooksArgs {
    #[command(subcommand)]
    command: Option<WebhooksCommand>,
}

#[derive(Subcommand)]
enum WebhooksCommand {
    /// Add a webhook. Its signing secret is shown once.
    Add {
        /// The HTTPS address to call.
        url: String,
        /// Only these events, comma-separated, such as deploy.failed. Default: every event.
        #[arg(long, value_delimiter = ',', value_name = "EVENTS")]
        events: Vec<String>,
        /// Only events about this project.
        #[arg(long, short = 'p', value_name = "PROJECT")]
        project: Option<String>,
    },
    /// Remove a webhook.
    Remove {
        /// Webhook id.
        id: String,
    },
    /// Give a webhook a new signing secret; the old one stops working now.
    Rotate {
        /// Webhook id.
        id: String,
    },
    /// Recent calls to a webhook and how they went.
    Deliveries {
        /// Webhook id.
        id: String,
    },
}

pub(crate) async fn webhooks(context: &Context, args: WebhooksArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    let check = |id: &str| {
        uuid::Uuid::parse_str(id)
            .map(|_| ())
            .or_else(|_| ui::usage("Use a webhook's id from hoist webhooks."))
    };
    match args.command {
        None => {
            let found = client.get_json("/webhooks").await?;
            ui::list(
                &found,
                "No webhooks. Add one with hoist webhooks add URL.",
                |item| {
                    vec![
                        ui::text(item, "url"),
                        ui::text(item, "events"),
                        ui::text(item, "project_id"),
                        ui::when(item, "created_at"),
                        ui::text(item, "id"),
                    ]
                },
                &["URL", "EVENTS", "PROJECT", "ADDED", "ID"],
            )
        }
        Some(WebhooksCommand::Add {
            url,
            events,
            project,
        }) => {
            let project_id = match project {
                Some(project) => Some(
                    context::project(context, &client, Some(&project), "")
                        .await?
                        .id,
                ),
                None => None,
            };
            let made = client
                .post_json(
                    "/webhooks",
                    &json!({"url": url, "events": events, "project_id": project_id}),
                )
                .await?;
            ui::show(&made, |made| {
                ui::done(&format!("Added. Id {}.", ui::text(made, "webhook.id")));
                eprintln!(
                    "Signing secret, shown once (keep it with the receiver): {}",
                    ui::text(made, "secret")
                );
            })
        }
        Some(WebhooksCommand::Remove { id }) => {
            check(&id)?;
            if !ui::confirm("Remove this webhook?", false)? {
                return Ok(());
            }
            let removed = client
                .delete_json(&context::path(&["webhooks", &id]))
                .await?;
            ui::show(&removed, |_| {
                ui::done("Removed. Its secret no longer signs anything.")
            })
        }
        Some(WebhooksCommand::Rotate { id }) => {
            check(&id)?;
            let rotated = client
                .post_json(&context::path(&["webhooks", &id, "rotate"]), &json!({}))
                .await?;
            ui::show(&rotated, |rotated| {
                ui::done("Rotated. The old secret stopped working.");
                eprintln!(
                    "New signing secret, shown once: {}",
                    ui::text(rotated, "secret")
                );
            })
        }
        Some(WebhooksCommand::Deliveries { id }) => {
            check(&id)?;
            let found = client
                .get_json(&context::path(&["webhooks", &id, "deliveries"]))
                .await?;
            ui::list(
                &found,
                "No calls yet.",
                |item| {
                    vec![
                        ui::when(item, "at"),
                        ui::text(item, "event"),
                        ui::text(item, "status"),
                        ui::text(item, "attempts"),
                    ]
                },
                &["WHEN", "EVENT", "STATUS", "ATTEMPTS"],
            )
        }
    }
}

// hoist settings

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct SettingsArgs {
    #[command(subcommand)]
    command: Option<SettingsCommand>,
}

#[derive(Subcommand)]
enum SettingsCommand {
    /// Print one setting.
    Get {
        /// The setting, such as backup_keep.
        key: String,
    },
    /// Change one setting.
    Set {
        /// The setting, such as backup_keep.
        key: String,
        /// Its new value.
        value: String,
    },
}

/// The settings people change, with what each one does.
const SETTINGS: [(&str, &str); 12] = [
    ("agents_enabled", "Agents may connect and act"),
    (
        "default_access",
        "The access a new agent's consent page starts on: view, ask or full",
    ),
    (
        "agent_expiry_days",
        "Days before an unused agent's access ends",
    ),
    ("idle_days", "Days without use before an agent is paused"),
    ("backup_every_hours", "Hours between automatic backups"),
    ("backup_keep", "How many automatic backups to keep"),
    ("history_days", "Days of activity to keep"),
    ("check_updates", "Check for new versions"),
    ("crash_reports", "Send crash reports"),
    ("network_page", "Serve Hoista's page on the private network"),
    (
        "mcp_http",
        "Serve MCP over HTTP on port 8788, for web agents such as ChatGPT",
    ),
    (
        "unisolated_builds",
        "Build without isolation where the provider's tools need it",
    ),
];

fn key(raw: &str) -> anyhow::Result<&'static str> {
    let wanted = raw.replace('-', "_");
    SETTINGS
        .iter()
        .find(|(name, _)| *name == wanted)
        .map(|(name, _)| *name)
        .ok_or_else(|| {
            ui::Usage(format!(
                "There's no setting {}. Settings: {}.",
                ui::clean(raw),
                SETTINGS
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
            .into()
        })
}

pub(crate) async fn settings(context: &Context, args: SettingsArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    let current = client.get_json("/settings").await?;
    match args.command {
        None => ui::show(&current, |current| {
            let mut table = ui::Table::new(&["SETTING", "VALUE", "WHAT IT DOES"]);
            for (name, what) in SETTINGS {
                table.row(vec![name.into(), ui::text(current, name), what.into()]);
            }
            table.print();
        }),
        Some(SettingsCommand::Get { key }) => {
            let key = self::key(&key)?;
            ui::show(&json!({key: current[key]}), |_| {
                println!("{}", ui::text(&current, key))
            })
        }
        Some(SettingsCommand::Set { key, value }) => {
            let key = self::key(&key)?;
            let parsed = match &current[key] {
                Value::Bool(_) => match value.as_str() {
                    "true" | "on" | "yes" => json!(true),
                    "false" | "off" | "no" => json!(false),
                    _ => return ui::usage(format!("{key} is true or false.")),
                },
                Value::Number(_) => json!(
                    value
                        .parse::<u16>()
                        .or_else(|_| ui::usage(format!("{key} is a whole number.")))?
                ),
                _ => json!(value),
            };
            let saved = client
                .patch_json("/settings", &json!({key: parsed}))
                .await?;
            ui::show(&saved, |saved| {
                ui::done(&format!("{key} is {}.", ui::text(saved, key)))
            })
        }
    }
}

// hoist profile

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct ProfileArgs {
    #[command(subcommand)]
    command: Option<ProfileCommand>,
}

#[derive(Subcommand)]
enum ProfileCommand {
    /// Set your name and initials.
    Set {
        /// Your name.
        #[arg(long)]
        name: String,
        /// Up to three initials.
        #[arg(long)]
        initials: String,
    },
}

async fn local_store(paths: &StatePaths) -> anyhow::Result<Option<SqliteStore>> {
    match SqliteStore::open(paths).await {
        Ok(store) => Ok(Some(store)),
        Err(StoreError::Filesystem(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

pub(crate) async fn profile(context: &Context, args: ProfileArgs) -> anyhow::Result<()> {
    let update = match args.command {
        Some(ProfileCommand::Set { name, initials }) => Some(Profile::new(name, initials)?),
        None => None,
    };
    let result: ProfileDto = if context.target()?.is_some() {
        let client = context.client().await?;
        match update {
            Some(profile) => client.save_profile(&profile.into()).await?,
            None => client.profile().await?,
        }
    } else if let Some(store) = local_store(&context.paths).await? {
        let operation = async {
            if let Some(profile) = &update {
                store.save_profile(profile).await?;
            }
            store
                .profile()
                .await?
                .map(ProfileDto::from)
                .context("Set your profile with hoist profile set --name NAME --initials XY.")
        }
        .await;
        store.close().await;
        operation?
    } else {
        let client = LocalClient::new(&context.paths)?;
        match update {
            Some(profile) => client.save_profile(&profile.into()).await?,
            None => client.profile().await?,
        }
    };
    let value = serde_json::to_value(&result)?;
    ui::show(&value, |value| {
        ui::fields(&[
            ("Name", ui::text(value, "name")),
            ("Initials", ui::text(value, "initials")),
        ]);
    })
}

// this computer

pub(crate) async fn start(context: &Context) -> anyhow::Result<()> {
    let daemon = crate::bootstrap::ensure(&context.paths).await?;
    let value = serde_json::to_value(&daemon.health)?;
    ui::show(&value, |value| {
        ui::done(&format!(
            "Hoista {} is running (process {}).",
            ui::text(value, "version"),
            ui::text(value, "pid")
        ));
    })
}

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct DoctorArgs {
    #[command(subcommand)]
    command: Option<DoctorCommand>,
}

#[derive(Subcommand)]
enum DoctorCommand {
    /// The key that encrypts environment values: check it, or set it up again.
    EnvironmentKey {
        /// Set it up again, with the review token this command printed.
        #[arg(long, value_name = "TOKEN")]
        retry_setup: Option<String>,
    },
}

pub(crate) async fn doctor(context: &Context, args: DoctorArgs) -> anyhow::Result<bool> {
    if let Some(DoctorCommand::EnvironmentKey { retry_setup }) = args.command {
        let client = context.client().await?;
        let result = match retry_setup {
            Some(token) => {
                if token.len() != 64
                    || !token
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return ui::usage("Use the review token hoist doctor environment-key printed.");
                }
                client
                    .post_json(
                        "/recovery/environment-key/retry-setup",
                        &json!({"review_token": token}),
                    )
                    .await?
            }
            None => client.get_json("/recovery/environment-key").await?,
        };
        ui::show(&result, |result| {
            println!(
                "{}",
                serde_json::to_string_pretty(result).unwrap_or_default()
            )
        })?;
        return Ok(true);
    }
    let report = if context.target()?.is_some() {
        context.client().await?.doctor().await?
    } else if let Some(store) = local_store(&context.paths).await? {
        let permit = Arc::new(tokio::sync::Semaphore::new(1))
            .acquire_owned()
            .await?;
        let report = hoista_server::doctor::inspect(&store, "direct", permit).await;
        let engine = hoista_server::wiring::engine(store.clone(), &context.paths)?;
        let report = hoista_server::doctor::accounts(report, &engine).await;
        drop(engine);
        store.close().await;
        report
    } else {
        LocalClient::new(&context.paths)?.doctor().await?
    };
    let healthy = report.healthy();
    let value = serde_json::to_value(&report)?;
    ui::show(&value, |value| {
        for check in value["checks"].as_array().into_iter().flatten() {
            let ok = check["ok"] == true || check["status"] == "ok" || check["healthy"] == true;
            let mark = if ok {
                console::style("✓").green()
            } else {
                console::style("✗").red()
            };
            println!(
                "{mark} {}  {}",
                ui::text(check, "name"),
                check["message"]
                    .as_str()
                    .or_else(|| check["detail"].as_str())
                    .map(ui::clean)
                    .unwrap_or_default()
            );
        }
        if value["checks"].is_null() {
            println!(
                "{}",
                serde_json::to_string_pretty(value).unwrap_or_default()
            );
        }
        if healthy {
            ui::done("Everything checks out.");
        } else {
            ui::warn("Something needs fixing; see above.");
        }
    })?;
    Ok(healthy)
}

#[derive(Args)]
pub(crate) struct DiagnosticsArgs {
    /// Save the report to a file in Hoista's state folder too.
    #[arg(long)]
    save: bool,
}

pub(crate) async fn diagnostics(context: &Context, args: DiagnosticsArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    let report = if args.save {
        client.post_json("/diagnostics", &json!({})).await?
    } else {
        client.get_json("/diagnostics").await?
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

#[derive(Args)]
pub(crate) struct DaemonArgs {
    #[command(subcommand)]
    command: DaemonCommand,
}

#[derive(Subcommand)]
enum DaemonCommand {
    /// Whether it's running, and what it's doing.
    Status,
    /// Restart it once the work under way is done.
    Restart,
}

pub(crate) async fn daemon(context: &Context, args: DaemonArgs) -> anyhow::Result<()> {
    match args.command {
        DaemonCommand::Status => {
            let status = LocalClient::new(&context.paths)?
                .get_json("/daemon")
                .await
                .context("Hoista isn't running on this computer. Start it with hoist start.")?;
            ui::show(&status, |status| {
                println!(
                    "{}",
                    serde_json::to_string_pretty(status).unwrap_or_default()
                )
            })
        }
        DaemonCommand::Restart => {
            let status = crate::bootstrap::restart(&context.paths, false).await?;
            let value = serde_json::to_value(&status)?;
            ui::show(&value, |_| ui::done("Restarted."))
        }
    }
}

#[derive(Args)]
pub(crate) struct ServiceArgs {
    #[command(subcommand)]
    command: ServiceCommand,
}

#[derive(Subcommand)]
enum ServiceCommand {
    /// Start Hoista at login, and again if it stops.
    Install {
        /// Print what would be installed, and install nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Restart the service.
    Restart,
    /// Whether the service is installed and running.
    Status,
    /// Stop starting Hoista at login.
    Uninstall,
}

pub(crate) async fn service(context: &Context, args: ServiceArgs) -> anyhow::Result<()> {
    let action = match args.command {
        ServiceCommand::Install { dry_run: true } => "preview",
        ServiceCommand::Install { dry_run: false } => "install",
        ServiceCommand::Restart => "restart",
        ServiceCommand::Status => "status",
        ServiceCommand::Uninstall => "uninstall",
    };
    crate::service::run(&context.paths, action).await
}

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct ToolsArgs {
    #[command(subcommand)]
    command: Option<ToolsCommand>,
}

#[derive(Subcommand)]
enum ToolsCommand {
    /// Save where provider tools are, from this shell's PATH.
    Configure,
}

pub(crate) fn tools(context: &Context, args: ToolsArgs) -> anyhow::Result<()> {
    let configure = matches!(args.command, Some(ToolsCommand::Configure));
    let configuration = if configure {
        Some(hoista_server::infrastructure::tool_paths::configure(
            &context.paths,
            true,
        )?)
    } else {
        hoista_server::infrastructure::tool_paths::read(&context.paths)?
    };
    let value = json!({
        "configured": configuration.is_some(),
        "configuration": configuration.as_ref(),
        "executables": configuration.as_ref().map(|value| value.executables()),
    });
    ui::show(&value, |value| {
        if value["configured"] == false {
            ui::note(
                "Not saved yet. Run hoist tools configure in a shell where your provider tools are on PATH.",
            );
            return;
        }
        let mut table = ui::Table::new(&["TOOL", "PATH"]);
        if let Some(found) = value["executables"].as_object() {
            for (tool, path) in found {
                table.row(vec![ui::clean(tool), ui::text(path, "")]);
            }
        }
        table.print();
    })?;
    if configure {
        ui::done(
            "Saved. A running engine picks this up when it next starts: hoist daemon restart.",
        );
    }
    Ok(())
}

#[derive(Args)]
pub(crate) struct SchemaArgs {
    /// Print TypeScript types instead.
    #[arg(long)]
    typescript: bool,
}

pub(crate) fn schema(args: &SchemaArgs) -> anyhow::Result<()> {
    if args.typescript {
        print!("{}", contracts::typescript());
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&contracts::json_schema())?
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_names_take_dashes_or_underscores() {
        assert_eq!(key("backup-keep").unwrap(), "backup_keep");
        assert_eq!(key("agents_enabled").unwrap(), "agents_enabled");
        assert!(key("token").is_err());
    }

    #[test]
    fn unknown_settings_are_a_usage_error() {
        let error = key("nope").unwrap_err();
        assert!(error.downcast_ref::<ui::Usage>().is_some());
    }
}
