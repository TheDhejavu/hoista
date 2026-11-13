//! `hoist remote`: other Hoistas you use (on a server, in Docker, at a provider),
//! like `git remote`. The list is shared with the desktop app. Signing in goes
//! through the remote's page in a browser, or a typed code with `--no-browser`.

use crate::{
    context::{self, Context, HERE, RemoteRecord, key_name},
    loopback::Loopback,
    ui,
};
use anyhow::{Context as _, bail};
use base64::Engine as _;
use clap::{Args, Subcommand, ValueEnum};
use hoista_server::{
    application::CredentialStore, client::server_origin, domain::Secret,
    infrastructure::credentials::SystemKeychain,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, time::Duration};

const MAX_NAME_CHARS: usize = 40;
const MAX_DEVICE_NAME_CHARS: usize = 60;
const CALL_TIMEOUT: Duration = Duration::from_secs(20);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(4);
/// How long the person has to finish signing in in the browser.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// How often, and how many times, the CLI checks whether the person approved it.
const APPROVAL_POLL: Duration = Duration::from_secs(2);
const APPROVAL_POLLS: usize = 300;
/// Waiting for a new server to come up.
const UP_CHECK_EVERY: Duration = Duration::from_secs(10);
const UP_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct RemoteArgs {
    #[command(subcommand)]
    command: Option<RemoteCommand>,
}

#[derive(Clone, Copy, ValueEnum)]
enum Where {
    /// In a Railway account you connected.
    Railway,
    /// In a Fly.io organization you connected.
    Fly,
    /// In Docker, on this computer or a server, from a copy of Hoista's repository.
    Docker,
}

#[derive(Subcommand)]
enum RemoteCommand {
    /// Check a Hoista's address: that it answers there, and whether it has an owner yet.
    Check {
        #[arg(help = "Its address, such as https://hoista.example.com, or a remote's name")]
        target: String,
    },
    /// Add a remote that's already running, and sign in to it.
    Add {
        /// What to call it, such as office.
        name: String,
        #[arg(help = "Its address, such as https://hoista.example.com")]
        url: String,
        /// Add it without signing in now.
        #[arg(long)]
        no_login: bool,
    },
    /// Put Hoista on Railway, Fly.io or Docker, and add it as a remote.
    Create(CreateArgs),
    /// Sign this computer in to a remote.
    Login {
        /// Remote name. Default: the one in use.
        name: Option<String>,
        /// Show a code to type on the remote's page instead of opening a browser.
        #[arg(long)]
        no_browser: bool,
    },
    /// Sign this computer out of a remote.
    Logout {
        /// Remote name. Default: the one in use.
        name: Option<String>,
    },
    /// Use a remote for the commands that follow, or this-computer for this one.
    Use {
        /// Remote name, or this-computer.
        name: Option<String>,
    },
    /// Rename a remote.
    Rename {
        /// Its name now.
        name: String,
        /// The new name.
        new_name: String,
    },
    /// Remove a remote from the list. It keeps running.
    Remove {
        /// Remote name.
        name: String,
    },
    /// Send this computer's projects and accounts to an empty remote.
    Push {
        /// Remote name. Default: the one in use.
        name: Option<String>,
    },
}

/// `hoist remote create`.
#[derive(Args)]
struct CreateArgs {
    /// Where it runs.
    #[arg(value_enum)]
    place: Option<Where>,
    /// Railway or Fly: the account to run it in.
    #[arg(long, value_name = "ACCOUNT")]
    account: Option<String>,
    /// What to call it. Default: from its address.
    #[arg(long, value_name = "NAME")]
    name: Option<String>,
    /// Railway or Fly: build from this GitHub repository (OWNER/NAME) instead of the released image.
    #[arg(long, value_name = "OWNER/NAME")]
    repo: Option<String>,
    /// With --repo: the branch.
    #[arg(long, requires = "repo")]
    branch: Option<String>,
    /// Docker: the copy of Hoista's repository. Default: this folder.
    #[arg(long, value_name = "PATH")]
    folder: Option<PathBuf>,
    #[arg(
        long,
        value_name = "URL",
        help = "Docker on a server: the HTTPS address it answers on. Default: http://localhost:8787"
    )]
    url: Option<String>,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_CHARS
        && name != HERE
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn signed_in(remote: &RemoteRecord) -> bool {
    SystemKeychain.get(&key_name(&remote.id)).is_ok()
}

pub(crate) async fn remote(context: &Context, args: RemoteArgs) -> anyhow::Result<bool> {
    match args.command {
        None => list(context).await,
        Some(RemoteCommand::Check { target }) => {
            let url = context
                .remotes()?
                .into_iter()
                .find(|remote| remote.name == target)
                .map_or(target, |remote| remote.url);
            let found = check(&url).await?;
            ui::show(&found, |found| {
                ui::fields(&[
                    ("Address", ui::text(found, "address")),
                    ("Hoista", "answers here".into()),
                    (
                        "Owner",
                        if found["claimed"] == true {
                            "set".into()
                        } else {
                            format!(
                                "none yet: find the claim code in its log, then run hoist remote add NAME {}",
                                ui::text(found, "address")
                            )
                        },
                    ),
                    ("For agents", ui::text(found, "mcp")),
                ]);
            })?;
            Ok(true)
        }
        Some(RemoteCommand::Add {
            name,
            url,
            no_login,
        }) => {
            if !valid_name(&name) {
                return ui::usage("Use a name of letters, digits, - and _, up to 40 characters.");
            }
            let origin = server_origin(&url)?;
            // Stop here if nothing usable answers there, saying why.
            if check(&origin).await?["claimed"] != true {
                ui::note("It has no owner yet. Its page asks for the claim code from its log.");
            }
            let mut remotes = context.remotes()?;
            if remotes.iter().any(|remote| remote.name == name) {
                bail!("There's already a remote called {name}.");
            }
            if let Some(same) = remotes.iter().find(|remote| remote.url == origin) {
                bail!("{} is already in the list as {}.", origin, same.name);
            }
            let record = RemoteRecord {
                id: uuid::Uuid::new_v4().to_string(),
                name: name.clone(),
                url: origin,
                added_at: context::now(),
            };
            remotes.push(record.clone());
            context.save_remotes(remotes)?;
            ui::done(&format!("Added {name}."));
            if no_login {
                ui::note(&format!("Sign in with hoist remote login {name}."));
                return Ok(true);
            }
            login(&record, false).await?;
            Ok(true)
        }
        Some(RemoteCommand::Create(args)) => create(context, args).await,
        Some(RemoteCommand::Login { name, no_browser }) => {
            let remote = chosen(context, name.as_deref())?;
            login(&remote, no_browser).await?;
            Ok(true)
        }
        Some(RemoteCommand::Logout { name }) => {
            let remote = chosen(context, name.as_deref())?;
            let _ = SystemKeychain.delete(&key_name(&remote.id));
            ui::done(&format!("Signed out of {}.", remote.name));
            ui::note(&format!(
                "To stop this computer's key working too, remove it at {}/owner/devices.",
                remote.url
            ));
            Ok(true)
        }
        Some(RemoteCommand::Use { name }) => {
            let name = match name {
                Some(name) => name,
                None if ui::can_ask() => {
                    let mut names = vec![HERE.to_owned()];
                    names.extend(context.remotes()?.into_iter().map(|remote| remote.name));
                    names[ui::pick("Use", &names)?].clone()
                }
                None => return ui::usage("Name it: hoist remote use NAME, or this-computer."),
            };
            let using = context.use_remote(&name)?;
            ui::show(&json!({"current": using}), |_| {
                ui::done(&format!("Using {using}."))
            })?;
            Ok(true)
        }
        Some(RemoteCommand::Rename { name, new_name }) => {
            if !valid_name(&new_name) {
                return ui::usage("Use a name of letters, digits, - and _, up to 40 characters.");
            }
            let remote = context.remote_named(&name)?;
            let mut remotes = context.remotes()?;
            if remotes.iter().any(|item| item.name == new_name) {
                bail!("There's already a remote called {new_name}.");
            }
            for item in &mut remotes {
                if item.id == remote.id {
                    item.name.clone_from(&new_name);
                }
            }
            context.save_remotes(remotes)?;
            context.renamed_remote(&remote.name, &new_name)?;
            ui::done(&format!("Renamed {} to {new_name}.", remote.name));
            Ok(true)
        }
        Some(RemoteCommand::Remove { name }) => {
            let remote = context.remote_named(&name)?;
            if !ui::confirm(
                &format!(
                    "Remove {} from the list? It keeps running; this computer signs out of it.",
                    remote.name
                ),
                false,
            )? {
                return Ok(true);
            }
            let remotes = context
                .remotes()?
                .into_iter()
                .filter(|item| item.id != remote.id)
                .collect();
            context.save_remotes(remotes)?;
            context.forget_remote(&remote)?;
            let _ = SystemKeychain.delete(&key_name(&remote.id));
            ui::done(&format!("Removed {}.", remote.name));
            Ok(true)
        }
        Some(RemoteCommand::Push { name }) => {
            let remote = chosen(context, name.as_deref())?;
            push(context, &remote).await?;
            Ok(true)
        }
    }
}

/// The named remote, or the one in use.
fn chosen(context: &Context, name: Option<&str>) -> anyhow::Result<RemoteRecord> {
    match name {
        Some(name) => context.remote_named(name),
        None => match context.target()? {
            Some(remote) => Ok(remote),
            None if ui::can_ask() => {
                let remotes = context.remotes()?;
                if remotes.is_empty() {
                    bail!("There are no remotes. Add one with hoist remote add NAME URL.");
                }
                let names: Vec<String> = remotes.iter().map(|remote| remote.name.clone()).collect();
                Ok(remotes[ui::pick("Remote", &names)?].clone())
            }
            None => ui::usage("Name the remote."),
        },
    }
}

async fn list(context: &Context) -> anyhow::Result<bool> {
    let remotes = context.remotes()?;
    let current = context.target()?.map(|remote| remote.id);
    let mut rows = vec![json!({
        "name": HERE,
        "url": null,
        "current": current.is_none(),
        "signed_in": true,
        "answers": null,
    })];
    for remote in &remotes {
        let signed = signed_in(remote);
        let answers = match context::remote_client(&context.paths, remote) {
            Ok(client) => tokio::time::timeout(HEALTH_TIMEOUT, client.health())
                .await
                .ok()
                .and_then(Result::ok)
                .map(|health| health.version),
            Err(_) => None,
        };
        rows.push(json!({
            "name": remote.name,
            "url": remote.url,
            "id": remote.id,
            "current": current.as_deref() == Some(remote.id.as_str()),
            "signed_in": signed,
            "version": answers,
        }));
    }
    ui::show(&Value::Array(rows.clone()), |_| {
        let mut table = ui::Table::new(&["", "NAME", "ADDRESS", "SIGNED IN", "HOISTA"]);
        for row in &rows {
            table.row(vec![
                if row["current"] == true {
                    "*".into()
                } else {
                    String::new()
                },
                ui::text(row, "name"),
                if row["url"].is_null() {
                    "on this computer".into()
                } else {
                    ui::text(row, "url")
                },
                if row["url"].is_null() {
                    "-".into()
                } else {
                    ui::text(row, "signed_in")
                },
                if row["url"].is_null() {
                    hoista_server::VERSION.into()
                } else {
                    row["version"]
                        .as_str()
                        .map_or_else(|| "not answering".into(), ui::clean)
                },
            ]);
        }
        table.print();
        if remotes.is_empty() {
            ui::note(
                "\nAdd one with hoist remote add NAME URL, or make one with hoist remote create.",
            );
        }
    })?;
    Ok(true)
}

fn random() -> String {
    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn computer_name() -> String {
    ["HOSTNAME", "COMPUTERNAME", "HOST"]
        .iter()
        .find_map(|name| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .map_or_else(
            || "This computer".to_owned(),
            |name| {
                name.trim()
                    .trim_end_matches(".local")
                    .chars()
                    .take(MAX_DEVICE_NAME_CHARS)
                    .collect()
            },
        )
}

fn http() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(CALL_TIMEOUT)
        .user_agent(concat!("hoist/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

fn keep(remote: &RemoteRecord, key: &str) -> anyhow::Result<()> {
    SystemKeychain
        .set(&key_name(&remote.id), &Secret::new(key.to_owned()))
        .map_err(|error| anyhow::anyhow!("Couldn't keep the key in the keychain: {error}"))
}

/// Signs this computer in to `remote`, in the browser or with a code.
async fn login(remote: &RemoteRecord, no_browser: bool) -> anyhow::Result<()> {
    let http = http()?;
    let name = computer_name();
    if no_browser {
        return login_with_code(&http, remote, &name).await;
    }
    let probe = http
        .get(format!("{}/owner/devices/connect", remote.url))
        .send()
        .await
        .with_context(|| format!("Couldn't reach {} at {}", remote.name, remote.url))?;
    if probe.status().as_u16() == 404 {
        ui::note(&format!(
            "{} runs an older Hoista; signing in with a code instead.",
            remote.name
        ));
        return login_with_code(&http, remote, &name).await;
    }
    let verifier = random();
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    let state = random();
    let listening = Loopback::bind(0).await?;
    let back = format!("http://127.0.0.1:{}/done", listening.port()?);
    let link = format!(
        "{}/owner/devices/connect?{}",
        remote.url,
        context::query(&[
            ("name", &name),
            ("redirect", &back),
            ("challenge", &challenge),
            ("state", &state)
        ])
    );
    ui::note(&format!(
        "Sign in on {}'s page and click Allow. The first time, it asks for its claim code.",
        remote.name
    ));
    ui::open(&link);
    let outcome = std::sync::Arc::new(std::sync::Mutex::new(Err::<(), String>(
        "Nothing came back.".into(),
    )));
    listening
        .wait("/done", SIGN_IN_TIMEOUT, |params| {
            let http = http.clone();
            let back = back.clone();
            let verifier = verifier.clone();
            let state = state.clone();
            let outcome = outcome.clone();
            let set = move |value: Result<(), String>| {
                if let Ok(mut slot) = outcome.lock() {
                    *slot = value;
                }
            };
            async move {
                if params.get("state") != Some(&state) {
                    set(Err("That answer was for a different sign-in.".into()));
                    return (
                        false,
                        "This answer is for a different sign-in. Start again in the terminal."
                            .into(),
                    );
                }
                if params.contains_key("error") {
                    set(Err("You cancelled it on its page.".into()));
                    return (false, "You cancelled it. You can close this tab.".into());
                }
                let traded = http
                    .post(format!("{}/owner/devices/token", remote.url))
                    .json(&json!({
                        "code": params.get("code").cloned().unwrap_or_default(),
                        "verifier": verifier,
                        "redirect": back,
                    }))
                    .send()
                    .await;
                let reply: Option<Value> = match traded {
                    Ok(response) => response.json().await.ok(),
                    Err(_) => None,
                };
                match reply.as_ref().and_then(|reply| reply["key"].as_str()) {
                    Some(key) => match keep(remote, key) {
                        Ok(()) => {
                            set(Ok(()));
                            (
                                true,
                                "Go back to the terminal. You can close this tab.".into(),
                            )
                        }
                        Err(error) => {
                            set(Err(error.to_string()));
                            (false, error.to_string())
                        }
                    },
                    None => {
                        let message = reply
                            .as_ref()
                            .and_then(|reply| reply["message"].as_str())
                            .unwrap_or("The sign-in ended. Try again.")
                            .to_owned();
                        set(Err(message.clone()));
                        (false, message)
                    }
                }
            }
        })
        .await?;
    let result = outcome.lock().map_or_else(
        |_| Err("The sign-in failed.".to_owned()),
        |slot| slot.clone(),
    );
    result.map_err(anyhow::Error::msg)?;
    ui::done(&format!(
        "Signed in to {}. Use it with hoist remote use {}.",
        remote.name, remote.name
    ));
    Ok(())
}

/// With no browser: the remote shows a code to type on its page.
async fn login_with_code(
    http: &reqwest::Client,
    remote: &RemoteRecord,
    name: &str,
) -> anyhow::Result<()> {
    let started = http
        .post(format!("{}/owner/devices/start", remote.url))
        .json(&json!({"name": name}))
        .send()
        .await
        .with_context(|| format!("Couldn't reach {}", remote.name))?;
    let ok = started.status().is_success();
    let started: Value = started
        .json()
        .await
        .context("It didn't answer like Hoista")?;
    if !ok {
        bail!(
            "{}",
            started["message"]
                .as_str()
                .unwrap_or("It refused the sign-in.")
        );
    }
    let (Some(poll), Some(code)) = (started["poll"].as_str(), started["code"].as_str()) else {
        bail!("It didn't answer like Hoista.");
    };
    let poll = Secret::new(poll.to_owned());
    println!(
        "On any device, open {}/owner/devices, sign in, and allow \"{name}\" with this code:\n\n    {}\n",
        remote.url,
        ui::clean(code)
    );
    for _ in 0..APPROVAL_POLLS {
        tokio::time::sleep(APPROVAL_POLL).await;
        let Ok(reply) = http
            .post(format!("{}/owner/devices/poll", remote.url))
            .json(&json!({"poll": poll.expose()}))
            .send()
            .await
        else {
            continue;
        };
        match reply.status().as_u16() {
            202 => {}
            200 => {
                let reply: Value = reply.json().await?;
                keep(
                    remote,
                    reply["key"].as_str().context("It didn't send a key")?,
                )?;
                ui::done(&format!("Signed in to {}.", remote.name));
                return Ok(());
            }
            _ => {
                let reply: Value = reply.json().await.unwrap_or_default();
                bail!(
                    "{}",
                    reply["message"]
                        .as_str()
                        .unwrap_or("The sign-in ended. Run hoist remote login again.")
                );
            }
        }
    }
    bail!("Nobody allowed it within ten minutes. Run hoist remote login again.")
}

/// Waits up to 20 minutes for a new remote to answer.
/// What the Hoista at `url` says about itself: the address it answers at,
/// whether it has an owner, and where agents connect.
///
/// # Errors
/// Says what's wrong: nothing answering, a Hoista started with another address
/// (or a tunnel changing the host name), or something that isn't a Hoista.
async fn check(url: &str) -> anyhow::Result<Value> {
    let origin = server_origin(url)?;
    let response = http()?
        .get(format!("{origin}/status"))
        .send()
        .await
        .map_err(|_| {
            anyhow::anyhow!("Nothing answered at {origin}. Check the address, and that Hoista and any tunnel in front of it are running.")
        })?;
    match response.status().as_u16() {
        200 => {}
        421 => {
            let answers_at = response
                .headers()
                .get("x-hoista-address")
                .and_then(|value| value.to_str().ok())
                .map(ui::clean);
            bail!(match answers_at {
                Some(at) => format!(
                    "That Hoista answers at {at}, not {origin}. To use {origin}, restart it with HOISTA_PUBLIC_URL={origin}."
                ),
                None => format!(
                    "{origin} refused the address. If a tunnel or proxy is in front of Hoista, make it pass the host name through."
                ),
            });
        }
        404 => bail!(
            "{origin} answered, but not as a Hoista, or as an older one. Check the address, or update Hoista there."
        ),
        status => {
            bail!("{origin} answered with {status}. Check it's your Hoista and that it's running.")
        }
    }
    let about: Value = response
        .json()
        .await
        .with_context(|| format!("{origin} answered, but not as a Hoista."))?;
    if about["hoista"] != true {
        bail!("{origin} answered, but not as a Hoista.");
    }
    Ok(json!({
        "address": origin,
        "claimed": about["claimed"] == true,
        "mcp": format!("{origin}/mcp"),
    }))
}

async fn wait_until_up(url: &str) -> bool {
    let Ok(client) = reqwest::Client::builder().timeout(UP_CHECK_TIMEOUT).build() else {
        return false;
    };
    for _ in 0..120 {
        if client
            .get(format!("{url}/healthz"))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return true;
        }
        tokio::time::sleep(UP_CHECK_EVERY).await;
    }
    false
}

fn add_record(context: &Context, name: Option<String>, url: &str) -> anyhow::Result<RemoteRecord> {
    let origin = server_origin(url)?;
    let mut remotes = context.remotes()?;
    if let Some(existing) = remotes.iter().find(|remote| remote.url == origin) {
        return Ok(existing.clone());
    }
    let name = name.unwrap_or_else(|| context::suggested_name(&origin, &remotes));
    if !valid_name(&name) || remotes.iter().any(|remote| remote.name == name) {
        return ui::usage(format!(
            "Pick another --name; {} is taken or not a name.",
            ui::clean(&name)
        ));
    }
    let record = RemoteRecord {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        url: origin,
        added_at: context::now(),
    };
    remotes.push(record.clone());
    context.save_remotes(remotes)?;
    Ok(record)
}

async fn create(context: &Context, args: CreateArgs) -> anyhow::Result<bool> {
    let CreateArgs {
        place,
        account,
        name,
        repo,
        branch,
        folder,
        url,
    } = args;
    let place = match place {
        Some(place) => place,
        None if ui::can_ask() => {
            let options = [
                "Railway".to_owned(),
                "Fly.io".to_owned(),
                "Docker".to_owned(),
            ];
            [Where::Railway, Where::Fly, Where::Docker][ui::pick("Where", &options)?]
        }
        None => return ui::usage("Say where: hoist remote create railway, fly or docker."),
    };
    if let Where::Docker = place {
        let folder = match folder {
            Some(folder) => folder,
            None => std::env::current_dir()?,
        };
        let script = folder.join("deploy/server/start.sh");
        if !script.is_file() {
            bail!(
                "That isn't a copy of Hoista's repository. Get one, then run this inside it:\n  git clone https://github.com/TheDhejavu/hoista.git && cd hoista\n  hoist remote create docker"
            );
        }
        let address = match &url {
            Some(url) => server_origin(url)?,
            None => "http://localhost:8787".to_owned(),
        };
        let mut command = std::process::Command::new("bash");
        command.arg(&script).current_dir(&folder);
        if url.is_some() {
            command.env("HOISTA_PUBLIC_URL", &address);
        }
        let status = command
            .status()
            .context("Couldn't run deploy/server/start.sh")?;
        if !status.success() {
            bail!("Hoista didn't start in Docker. Its output is above.");
        }
        let record = add_record(context, name, &address)?;
        ui::done(&format!("Added it as {}.", record.name));
        if url.is_some() && !address.starts_with("https://") {
            ui::note("Passkeys need HTTPS: put a reverse proxy in front before signing in.");
        }
        login(&record, false).await?;
        return Ok(true);
    }
    let local = context.local().await?;
    let wanted = match place {
        Where::Railway => "railway",
        _ => "fly",
    };
    let only = |account: &Value| account["provider"] == wanted;
    let account = crate::accounts::account(
        &local,
        account.as_deref(),
        Some(&only),
        &format!("Pass --account with the {wanted} account to run it in."),
    )
    .await?;
    ui::note(&format!(
        "Putting Hoista on {}. The provider may ask you to confirm it's you.",
        ui::text(&account, "label")
    ));
    let made = local
        .post_json_long_running(
            "/server",
            &json!({"account_id": account["id"], "repo": repo, "branch": branch}),
        )
        .await?;
    let address = made["url"]
        .as_str()
        .context("It didn't say its address")?
        .to_owned();
    ui::note(&format!(
        "Building it at {address}. The first build takes a few minutes."
    ));
    if !wait_until_up(&address).await {
        ui::warn("It's still building. When it answers, sign in with hoist remote login.");
    }
    let record = add_record(context, name, &address)?;
    ui::done(&format!("Added it as {}.", record.name));
    if let Some(code) = made["claim_link"]
        .as_str()
        .and_then(|link| link.split("#code=").nth(1))
    {
        println!("\n  Claim code: {}\n", ui::clean(code));
    }
    if let Some(help) = made["seal_help"].as_str() {
        ui::warn(&format!("One more step: {}", ui::clean(help)));
        if let Some(page) = made["seal_page"].as_str() {
            println!("  {}", ui::clean(page));
        }
    }
    login(&record, false).await?;
    ui::note(&format!("Agents connect at {address}/mcp."));
    Ok(true)
}

/// Moves this computer's Hoista to an empty remote: an encrypted export
/// with a one-time passphrase, restored there, each project back in its
/// mode, then this computer's agents paused so the old copy stops taking changes.
async fn push(context: &Context, remote: &RemoteRecord) -> anyhow::Result<()> {
    let server = context::remote_client(&context.paths, remote)?;
    let local = context.local().await?;
    server
        .health()
        .await
        .map_err(|error| anyhow::anyhow!("{} didn't answer: {error}", remote.name))?;
    let count = |value: &Value| value["items"].as_array().map_or(0, Vec::len);
    let (there_projects, there_accounts) = (
        count(&server.get_json("/projects").await?),
        count(&server.get_json("/accounts").await?),
    );
    if there_projects > 0 || there_accounts > 0 {
        bail!(
            "{} already has projects or accounts. A push only goes into an empty remote; nothing is merged.",
            remote.name
        );
    }
    let projects = local.get_json("/projects").await?;
    let managed: Vec<(String, String)> = projects["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|project| project["mode"] == "managed")
        .map(|project| (ui::text(project, "id"), ui::text(project, "name")))
        .collect();
    if !ui::confirm(
        &format!(
            "Send {} projects and every account to {}? Agents on this computer are paused after.",
            count(&projects),
            remote.name
        ),
        true,
    )? {
        return Ok(());
    }
    let folder = context.paths.root().join("moves");
    std::fs::create_dir_all(&folder)?;
    let file = folder.join(format!("move-{}.hoista", uuid::Uuid::new_v4().simple()));
    let passphrase = Secret::new(random());
    ui::note("Packing up projects, accounts and their sign-ins");
    let sent = async {
        local
            .post_json_long_running(
                "/transfer/export",
                &json!({"path": file, "passphrase": passphrase.expose()}),
            )
            .await?;
        let data = base64::engine::general_purpose::STANDARD.encode(std::fs::read(&file)?);
        ui::note(&format!("Sending it to {}", remote.name));
        server
            .post_json_long_running(
                "/transfer/receive",
                &json!({"passphrase": passphrase.expose(), "data": data}),
            )
            .await
            .map_err(anyhow::Error::from)
    }
    .await;
    let _ = std::fs::remove_file(&file);
    let restored = sent?;
    let mut left_watching = Vec::new();
    for (id, name) in &managed {
        if server
            .put_json(
                &context::path(&["projects", id, "mode"]),
                &json!({"mode": "managed"}),
            )
            .await
            .is_err()
        {
            left_watching.push(name.clone());
        }
    }
    local
        .patch_json("/settings", &json!({"agents_enabled": false}))
        .await?;
    ui::done(&format!(
        "{} has it: {} sign-ins restored.",
        remote.name,
        restored["credentials_staged"].as_u64().unwrap_or(0)
    ));
    if !left_watching.is_empty() {
        ui::warn(&format!(
            "These stay watching there until you manage them: {}",
            left_watching.join(", ")
        ));
    }
    ui::note(&format!(
        "Agents on this computer are paused (undo: hoist agents resume). Use the remote with hoist remote use {}.",
        remote.name
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_names_are_short_and_not_this_computer() {
        assert!(valid_name("office"));
        assert!(valid_name("vps_2"));
        assert!(!valid_name(HERE));
        assert!(!valid_name("has space"));
        assert!(!valid_name(""));
    }

    #[test]
    fn the_challenge_secret_is_long_and_url_safe() {
        let secret = random();
        assert!(secret.len() >= 43);
        assert!(
            secret
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        assert_ne!(random(), secret);
    }
}
