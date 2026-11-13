//! Coding agents: who's connected, what each may do, and cutting them off.
//! Also `hoist mcp` (the server a coding tool starts) and `hoist skills`.

use crate::{
    context::{self, Context},
    ui,
};
use anyhow::{Context as _, bail};
use clap::{Args, Subcommand, ValueEnum};
use hoista_server::{client::LocalClient, infrastructure::paths::StatePaths};
use serde_json::{Value, json};

const MAX_NAME_BYTES: usize = 80;
/// How often the CLI checks that MCP over HTTP is listening.
const LISTEN_POLL: std::time::Duration = std::time::Duration::from_millis(300);

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct AgentsArgs {
    #[command(subcommand)]
    command: Option<AgentsCommand>,
}

#[derive(Clone, Copy, ValueEnum)]
enum Access {
    /// View only: read projects, logs and status.
    View,
    /// Ask for approval: make changes, and ask you before deleting, removing or connecting an account.
    Ask,
    /// Full access: change anything without asking.
    Full,
}

impl Access {
    /// The same levels as the desktop app: scopes, plus whether it asks first.
    fn scopes(self) -> &'static [&'static str] {
        match self {
            Self::View => &["apps:read"],
            Self::Ask | Self::Full => &["apps:read", "apps:write", "apps:create"],
        }
    }

    fn without_asking(self) -> bool {
        matches!(self, Self::Full)
    }
}

/// An agent's level, read the way the desktop app reads it.
fn access_of(scopes: &Value, without_asking: bool) -> &'static str {
    let writes = scopes
        .as_array()
        .is_some_and(|list| list.iter().any(|item| item == "apps:write"));
    match (writes, without_asking) {
        (false, _) => "view",
        (true, false) => "ask",
        (true, true) => "full",
    }
}

#[derive(Subcommand)]
enum AgentsCommand {
    /// Coding tools asking to connect.
    Requests,
    /// Let a coding tool that asked connect.
    Allow {
        /// Request id. In a terminal, pick it from a list instead.
        request: Option<String>,
        /// What it may do.
        #[arg(long, value_enum)]
        access: Option<Access>,
        /// Only these projects (names or ids, comma-separated). Default: every project.
        #[arg(long, value_delimiter = ',', value_name = "PROJECTS")]
        projects: Vec<String>,
    },
    /// Turn down a coding tool that asked to connect.
    Deny {
        /// Request id.
        request: String,
    },
    /// Change what a connected agent may do and touch.
    Access {
        /// Agent name or id.
        agent: Option<String>,
        /// What it may do.
        #[arg(long, value_enum)]
        access: Option<Access>,
        /// Only these projects (names or ids, comma-separated).
        #[arg(
            long,
            value_delimiter = ',',
            value_name = "PROJECTS",
            conflicts_with = "all_projects"
        )]
        projects: Vec<String>,
        /// Every project.
        #[arg(long)]
        all_projects: bool,
        /// Its access ends at this time (RFC 3339), or "never".
        #[arg(long, value_name = "WHEN")]
        expires: Option<String>,
    },
    /// Pause every agent's access. Undo with hoist agents resume.
    Pause,
    /// Let agents in again after a pause.
    Resume,
    /// Cut every agent off now and pause access. Projects keep running.
    StopAll,
    /// Disconnect one agent. Its token stops working now.
    Revoke {
        /// Agent name or id.
        agent: Option<String>,
    },
    /// Give a local agent a new token; the old one stops working. Also reconnects a disconnected one.
    Rotate {
        /// Agent name or id.
        agent: Option<String>,
    },
}

async fn all(client: &LocalClient) -> anyhow::Result<Vec<Value>> {
    let page = client.get_json("/agents").await?;
    Ok(page["items"].as_array().cloned().unwrap_or_default())
}

async fn agent(client: &LocalClient, wanted: Option<&str>, how: &str) -> anyhow::Result<Value> {
    find_agent(client, wanted, how, false).await
}

/// An agent by name or id; with `revoked`, disconnected ones too.
async fn find_agent(
    client: &LocalClient,
    wanted: Option<&str>,
    how: &str,
    revoked: bool,
) -> anyhow::Result<Value> {
    let agents: Vec<Value> = all(client)
        .await?
        .into_iter()
        .filter(|agent| revoked || agent["revoked"] != true)
        .collect();
    if let Some(wanted) = wanted {
        let found: Vec<&Value> = agents
            .iter()
            .filter(|agent| {
                agent["id"] == wanted
                    || agent["name"]
                        .as_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
            })
            .collect();
        return match found.as_slice() {
            [one] => Ok((*one).clone()),
            [] => bail!(
                "There's no agent called {}. Run hoist agents.",
                ui::clean(wanted)
            ),
            _ => bail!(
                "More than one agent is called {}. Use its id.",
                ui::clean(wanted)
            ),
        };
    }
    if agents.is_empty() {
        bail!("No agents are connected.");
    }
    if !ui::can_ask() {
        return ui::usage(how.to_owned());
    }
    let names: Vec<String> = agents.iter().map(|agent| ui::text(agent, "name")).collect();
    Ok(agents[ui::pick("Agent", &names)?].clone())
}

async fn project_ids(
    context: &Context,
    client: &LocalClient,
    names: &[String],
) -> anyhow::Result<Option<Vec<String>>> {
    if names.is_empty() {
        return Ok(None);
    }
    let mut ids = Vec::new();
    for name in names {
        ids.push(context::project(context, client, Some(name), "").await?.id);
    }
    Ok(Some(ids))
}

pub(crate) async fn agents(context: &Context, args: AgentsArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    match args.command {
        None => {
            let settings = client.get_json("/settings").await.unwrap_or(Value::Null);
            let projects = context::projects(&client).await.unwrap_or_default();
            let names = |grant: &Value| match grant["projects"].as_array() {
                None => "all".to_owned(),
                Some(ids) => ids
                    .iter()
                    .map(|id| {
                        projects
                            .iter()
                            .find(|project| project.id == id.as_str().unwrap_or_default())
                            .map_or_else(|| ui::text(id, ""), |project| ui::clean(&project.name))
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
            };
            if settings["agents_enabled"] == false && !ui::mode().json {
                ui::warn("Agent access is paused. Let them in again with hoist agents resume.");
            }
            let agents: Vec<Value> = all(&client)
                .await?
                .into_iter()
                .filter(|agent| agent["revoked"] != true)
                .collect();
            ui::list(
                &Value::Array(agents),
                "No agents are connected. Add Hoista to a coding tool with hoist mcp config.",
                |item| {
                    vec![
                        ui::text(item, "name"),
                        access_of(&item["scopes"], item["grant"]["without_asking"] == true).into(),
                        names(&item["grant"]),
                        ui::when(item, "last_seen"),
                        ui::text(item, "transport"),
                        ui::text(item, "id"),
                    ]
                },
                &[
                    "NAME",
                    "ACCESS",
                    "PROJECTS",
                    "LAST SEEN",
                    "CONNECTS BY",
                    "ID",
                ],
            )
        }
        Some(AgentsCommand::Requests) => {
            let found = client.get_json("/agents/connection-requests").await?;
            ui::list(
                &found,
                "No coding tool is asking to connect.",
                |item| {
                    vec![
                        ui::text(item, "name"),
                        ui::text(item, "code"),
                        ui::when(item, "created_at"),
                        ui::text(item, "status"),
                        ui::text(item, "id"),
                    ]
                },
                &["NAME", "CODE", "ASKED", "STATUS", "ID"],
            )?;
            ui::note(
                "Check the code matches what the tool shows, then: hoist agents allow ID --access view|ask|full.",
            );
            Ok(())
        }
        Some(AgentsCommand::Allow {
            request,
            access,
            projects,
        }) => {
            let pending = client.get_json("/agents/connection-requests").await?;
            let waiting: Vec<Value> = pending["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|item| item["status"] == "pending")
                .cloned()
                .collect();
            let chosen = match request {
                Some(id) => waiting
                    .iter()
                    .find(|item| item["id"] == id.as_str())
                    .cloned()
                    .with_context(|| {
                        format!(
                            "No request {} is waiting. See hoist agents requests.",
                            ui::clean(&id)
                        )
                    })?,
                None if ui::can_ask() && !waiting.is_empty() => {
                    let labels: Vec<String> = waiting
                        .iter()
                        .map(|item| {
                            format!(
                                "{}  code {}",
                                ui::text(item, "name"),
                                ui::text(item, "code")
                            )
                        })
                        .collect();
                    waiting[ui::pick("Request", &labels)?].clone()
                }
                None if waiting.is_empty() => bail!("No coding tool is asking to connect."),
                None => {
                    return ui::usage(
                        "Name the request: hoist agents allow ID --access view|ask|full.",
                    );
                }
            };
            let access = match access {
                Some(access) => access,
                None if ui::can_ask() => {
                    let options = [
                        "View only".to_owned(),
                        "Ask for approval".to_owned(),
                        "Full access".to_owned(),
                    ];
                    [Access::View, Access::Ask, Access::Full][ui::pick("Access", &options)?]
                }
                None => return ui::usage("Say what it may do: --access view, ask or full."),
            };
            if !ui::mode().yes && ui::can_ask() {
                println!(
                    "{} shows the code {}.",
                    ui::text(&chosen, "name"),
                    ui::text(&chosen, "code")
                );
                if !ui::confirm("Does it match what the tool shows?", false)? {
                    ui::note(
                        "Nothing allowed. Deny it with hoist agents deny if you don't know it.",
                    );
                    return Ok(());
                }
            }
            let mut body = json!({
                "allow": true,
                "scopes": access.scopes(),
                "grant": {"without_asking": access.without_asking()},
            });
            if let Some(ids) = project_ids(context, &client, &projects).await? {
                body["grant"]["projects"] = json!(ids);
            }
            let answered = client
                .post_json(
                    &context::path(&["agents", "connection-requests", &ui::text(&chosen, "id")]),
                    &body,
                )
                .await?;
            ui::show(&answered, |_| {
                ui::done(&format!("{} can connect.", ui::text(&chosen, "name")))
            })
        }
        Some(AgentsCommand::Deny { request }) => {
            context::check_id(&request, "a request")?;
            let answered = client
                .post_json(
                    &context::path(&["agents", "connection-requests", &request]),
                    &json!({"allow": false}),
                )
                .await?;
            ui::show(&answered, |_| ui::done("Turned down."))
        }
        Some(AgentsCommand::Access {
            agent,
            access,
            projects,
            all_projects,
            expires,
        }) => {
            let agent = self::agent(
                &client,
                agent.as_deref(),
                "Name the agent: hoist agents access AGENT --access ask.",
            )
            .await?;
            let id = ui::text(&agent, "id");
            if access.is_none() && projects.is_empty() && !all_projects && expires.is_none() {
                return ui::usage(
                    "Say what to change: --access, --projects, --all-projects or --expires.",
                );
            }
            if let Some(access) = access {
                client
                    .put_json(
                        &context::path(&["agents", &id, "scopes"]),
                        &json!({"scopes": access.scopes()}),
                    )
                    .await?;
            }
            if access.is_some() || !projects.is_empty() || all_projects || expires.is_some() {
                let current = &agent["grant"];
                let mut grant = serde_json::Map::new();
                for key in [
                    "projects",
                    "environment_ids",
                    "deny",
                    "expires_at",
                    "without_asking",
                    "environment_rules",
                    "exceptions",
                ] {
                    if let Some(value) = current.get(key) {
                        grant.insert(key.into(), value.clone());
                    }
                }
                if let Some(access) = access {
                    grant.insert("without_asking".into(), json!(access.without_asking()));
                }
                if all_projects {
                    grant.insert("projects".into(), Value::Null);
                } else if let Some(ids) = project_ids(context, &client, &projects).await? {
                    grant.insert("projects".into(), json!(ids));
                }
                match expires.as_deref() {
                    Some("never") => {
                        grant.insert("expires_at".into(), Value::Null);
                    }
                    Some(when) => {
                        if chrono::DateTime::parse_from_rfc3339(when).is_err() {
                            return ui::usage("Write --expires as an RFC 3339 time, or never.");
                        }
                        grant.insert("expires_at".into(), json!(when));
                    }
                    None => {}
                }
                client
                    .put_json(
                        &context::path(&["agents", &id, "grant"]),
                        &Value::Object(grant),
                    )
                    .await?;
            }
            let updated = client.get_json("/agents").await?;
            let saved = updated["items"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|item| item["id"] == id.as_str())
                .cloned()
                .unwrap_or(Value::Null);
            ui::show(&saved, |_| {
                ui::done(&format!("Updated {}.", ui::text(&agent, "name")))
            })
        }
        Some(command @ (AgentsCommand::Pause | AgentsCommand::Resume)) => {
            let enabled = matches!(command, AgentsCommand::Resume);
            let settings = client
                .patch_json("/settings", &json!({"agents_enabled": enabled}))
                .await?;
            ui::show(&settings, |_| {
                ui::done(if enabled {
                    "Agents can connect again."
                } else {
                    "Agent access is paused."
                });
            })
        }
        Some(AgentsCommand::StopAll) => {
            if !ui::confirm("Cut off every agent and pause agent access?", false)? {
                return Ok(());
            }
            let stopped = client.post_json("/agents/stop-all", &json!({})).await?;
            ui::show(&stopped, |stopped| {
                ui::done(&format!(
                    "Cut off {} agents and paused access. Projects keep running.",
                    stopped["revoked"].as_u64().unwrap_or(0)
                ));
                ui::note(
                    "If a provider token may have leaked, rotate it: hoist accounts rotate ACCOUNT.",
                );
            })
        }
        Some(AgentsCommand::Revoke { agent }) => {
            let agent = find_agent(
                &client,
                agent.as_deref(),
                "Name the agent: hoist agents revoke AGENT --yes.",
                true,
            )
            .await?;
            if !ui::confirm(&format!("Disconnect {}?", ui::text(&agent, "name")), false)? {
                return Ok(());
            }
            let revoked = client
                .delete_json(&context::path(&["agents", &ui::text(&agent, "id")]))
                .await?;
            ui::show(&revoked, |_| {
                ui::done(&format!("Disconnected {}.", ui::text(&agent, "name")))
            })
        }
        Some(AgentsCommand::Rotate { agent }) => {
            let agent = find_agent(
                &client,
                agent.as_deref(),
                "Name the agent: hoist agents rotate AGENT.",
                true,
            )
            .await?;
            let rotated = client
                .post_json(
                    &context::path(&["agents", &ui::text(&agent, "id"), "reconnect"]),
                    &json!({}),
                )
                .await?;
            ui::show(&rotated, |_| {
                ui::done(&format!(
                    "{} has a new token; the old one no longer works. Restart the tool so it picks it up.",
                    ui::text(&agent, "name")
                ));
            })
        }
    }
}

// hoist mcp

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct McpArgs {
    #[command(subcommand)]
    command: Option<McpCommand>,
    /// The name the agent shows up as in Hoista.
    #[arg(long, value_name = "NAME")]
    name: Option<String>,
}

#[derive(Clone, Copy, ValueEnum)]
enum OnOff {
    On,
    Off,
}

#[derive(Subcommand)]
enum McpCommand {
    /// Print what to add to a coding tool's MCP settings.
    Config {
        /// The name the agent shows up as in Hoista.
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
    },
    /// MCP over HTTP, for ChatGPT and other web agents: on or off, and its address.
    ///
    /// Hoista serves MCP over HTTP on 127.0.0.1:8788. Point a tunnel or proxy
    /// there (Cloudflare Tunnel, ngrok, Tailscale Funnel, Caddy) and give the
    /// agent its https:// address with /mcp. Hoista learns the address from
    /// the first request from outside; you allow each agent on this computer.
    Http {
        /// Turn it on or off. Without it, shows whether it's on and its address.
        #[arg(value_enum)]
        state: Option<OnOff>,
        /// Forget its public address, to use a different tunnel or proxy.
        /// Agents connected at the old one sign in again.
        #[arg(long, conflicts_with_all = ["state", "url"])]
        forget: bool,
        /// Set its public address yourself, for a tunnel or proxy that
        /// doesn't pass it on, such as `https://hoista.example.com`.
        #[arg(long, value_name = "URL", conflicts_with = "state")]
        url: Option<String>,
    },
}

fn checked_name(name: Option<&str>) -> anyhow::Result<()> {
    if let Some(name) = name
        && (name.trim().is_empty()
            || name.len() > MAX_NAME_BYTES
            || name.chars().any(char::is_control))
    {
        return ui::usage("Use a --name of 1 to 80 characters without control characters.");
    }
    Ok(())
}

pub(crate) fn mcp_config(paths: &StatePaths, name: Option<&str>) -> anyhow::Result<Value> {
    checked_name(name)?;
    let binary = std::fs::canonicalize(std::env::current_exe()?)?;
    let root = std::path::absolute(paths.root())?;
    let mut args = vec![
        "mcp".to_owned(),
        "--state-dir".to_owned(),
        root.to_str()
            .context("The state folder's path isn't UTF-8")?
            .to_owned(),
    ];
    if let Some(name) = name {
        args.extend(["--name".into(), name.into()]);
    }
    Ok(json!({"mcpServers": {"hoista": {"command": binary, "args": args}}}))
}

pub(crate) async fn mcp(context: &Context, args: McpArgs) -> anyhow::Result<()> {
    match args.command {
        Some(McpCommand::Config { name }) => {
            let config = match context.target()? {
                // A remote: agents connect to its own address and sign in there.
                Some(remote) => {
                    json!({
                        "mcpServers": {
                            "hoista": {
                                "type": "http",
                                "url": format!("{}/mcp", remote.url),
                            },
                        },
                    })
                }
                None => mcp_config(&context.paths, name.as_deref())?,
            };
            println!("{}", serde_json::to_string_pretty(&config)?);
            Ok(())
        }
        Some(McpCommand::Http { state, forget, url }) => http(context, state, forget, url).await,
        None => {
            checked_name(args.name.as_deref())?;
            if context.target()?.is_some() {
                bail!(
                    "Agents connect to a remote at its own address. Print it with hoist mcp config."
                );
            }
            let _daemon = crate::bootstrap::ensure(&context.paths).await?;
            // The keychain entry for this tool's connection to Hoista.
            let vault =
                hoista_server::infrastructure::credentials::NativeVault::named("connections")
                    .traced(&context.paths.logs());
            hoista_server::stdio::run(
                &context.paths,
                args.name.as_deref(),
                std::sync::Arc::new(vault),
            )
            .await?;
            Ok(())
        }
    }
}

async fn http(
    context: &Context,
    state: Option<OnOff>,
    forget: bool,
    url: Option<String>,
) -> anyhow::Result<()> {
    if context.target()?.is_some() {
        bail!("A remote serves MCP over HTTP at its own address. Print it with hoist mcp config.");
    }
    let client = context.client().await?;
    if let Some(state) = state {
        let on = matches!(state, OnOff::On);
        client
            .patch_json("/settings", &json!({ "mcp_http": on }))
            .await?;
        if !on {
            if ui::mode().json {
                println!("{}", json!({ "mcp_http": false }));
            } else {
                ui::done("MCP over HTTP is off. Web agents can't reach this Hoista.");
            }
            return Ok(());
        }
        // It starts listening within a couple of seconds.
        for _ in 0..10 {
            let link = client.get_json("/agents/link").await?;
            if link.get("listening").and_then(Value::as_bool) == Some(true) {
                break;
            }
            tokio::time::sleep(LISTEN_POLL).await;
        }
    }
    let link = if forget {
        client.delete_json("/agents/link").await?
    } else if let Some(url) = &url {
        client
            .put_json("/agents/link", &json!({ "url": url }))
            .await?
    } else {
        client.get_json("/agents/link").await?
    };
    if ui::mode().json {
        println!("{}", serde_json::to_string_pretty(&link)?);
        return Ok(());
    }
    let local = ui::text(&link, "local");
    let on = client
        .get_json("/settings")
        .await?
        .get("mcp_http")
        .and_then(Value::as_bool)
        == Some(true);
    if !on {
        ui::note("MCP over HTTP is off. Turn it on with hoist mcp http on.");
        return Ok(());
    }
    if link.get("listening").and_then(Value::as_bool) != Some(true) {
        ui::note(&format!(
            "MCP over HTTP is on, but Hoista can't listen at {local}: another app is using port {}. Close that app.",
            hoista_server::link::PORT
        ));
        return Ok(());
    }
    match link.get("mcp").and_then(Value::as_str) {
        Some(mcp) => {
            if url.is_some() {
                ui::done("Set its address.");
            }
            ui::done(&format!("Web agents connect at {mcp}"));
            if link.get("seen_at").and_then(Value::as_str).is_some() {
                ui::note(&format!(
                    "Last request through it: {}.",
                    ui::when(&link, "seen_at")
                ));
            }
            ui::note(&format!(
                "Add it to ChatGPT (or another web agent) as a connector. Your tunnel or proxy points at {local}. To use a different one, run hoist mcp http --forget."
            ));
        }
        None => {
            if forget {
                ui::done("Forgot its address.");
            }
            ui::done(&format!("MCP over HTTP is on at {local}"));
            ui::note(
                "Point a tunnel or proxy at it, then add its https:// address followed by /mcp to ChatGPT (or another web agent). Hoista learns the address from the first request from outside, or set it with --url.",
            );
        }
    }
    Ok(())
}

// hoist skills

#[derive(Args)]
pub(crate) struct SkillsArgs {
    #[command(subcommand)]
    command: SkillsCommand,
}

#[derive(Subcommand)]
enum SkillsCommand {
    /// Install Hoista's skills into Claude Code and Codex, or only one.
    Install {
        /// Only Claude Code.
        #[arg(long, conflicts_with = "codex")]
        claude: bool,
        /// Only Codex.
        #[arg(long)]
        codex: bool,
    },
}

pub(crate) fn skills(args: SkillsArgs) -> anyhow::Result<()> {
    use hoista_server::{
        application::agents::tool::Tool,
        infrastructure::agent_setup::{home, set_up},
    };
    let SkillsCommand::Install { claude, codex } = args.command;
    let home = home().context("Couldn't find your home folder.")?;
    let mut done = Vec::new();
    for (only, tool) in [(claude, Tool::ClaudeCode), (codex, Tool::Codex)] {
        if only || (!claude && !codex) {
            done.extend(set_up(tool, &home)?);
        }
    }
    if ui::mode().json {
        println!("{}", serde_json::to_string_pretty(&done)?);
    } else {
        for line in done {
            ui::done(&line);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_levels_map_to_scopes_and_back() {
        for access in [Access::View, Access::Ask, Access::Full] {
            let scopes = json!(access.scopes());
            let name = access_of(&scopes, access.without_asking());
            assert_eq!(
                Access::from_str(name, true)
                    .ok()
                    .map(Access::without_asking),
                Some(access.without_asking())
            );
            assert_eq!(
                Access::from_str(name, true).ok().map(Access::scopes),
                Some(access.scopes())
            );
        }
    }

    #[test]
    fn mcp_configuration_keeps_arguments_apart_and_has_no_credentials() {
        let config = mcp_config(
            &StatePaths::new(Some("state with spaces".into())).unwrap(),
            Some("Code Tool"),
        )
        .unwrap();
        let server = &config["mcpServers"]["hoista"];
        assert!(std::path::Path::new(server["command"].as_str().unwrap()).is_absolute());
        assert_eq!(server["args"][0], "mcp");
        assert!(
            server["args"][2]
                .as_str()
                .unwrap()
                .ends_with("state with spaces")
        );
        assert_eq!(server["args"][4], "Code Tool");
        assert!(server.get("env").is_none());
        assert!(mcp_config(&StatePaths::new(None).unwrap(), Some("bad\nname")).is_err());
    }
}
