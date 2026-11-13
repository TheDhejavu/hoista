//! Hoista on a server as three processes: a supervisor, the credential keeper
//! and the gateway (agents, pages, API, deploys).
//!
//! Only the keeper gets the sealing key; the gateway starts with no secrets in
//! its environment. Started as root, the keeper and gateway get their own users.

#![cfg(unix)]

use anyhow::{Context, bail};
use hoista_server::infrastructure::{paths::StatePaths, token_store};
use std::{
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
    time::Duration,
};
use tokio::io::AsyncWriteExt as _;

/// How long the keeper gets to open its socket.
const START_TIMEOUT: Duration = Duration::from_secs(15);
const START_POLL: Duration = Duration::from_millis(50);
/// How long the keeper gets to stop when asked.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
const EXIT_TIMEOUT: Duration = Duration::from_secs(30);

const KEEPER_USER: &str = "hoista-keeper";
const GATEWAY_USER: &str = "hoista";

/// What the keeper's environment may carry beyond `PATH`: the choice of
/// token store and what an outside store would need to sign in.
const KEEPER_VARIABLES: &[&str] = &[
    token_store::TOKEN_STORE_VARIABLE,
    "OP_SERVICE_ACCOUNT_TOKEN",
    "OP_ACCOUNT",
    "OP_CONNECT_PROVIDER",
    "OP_CONNECT_TOKEN",
];

/// Never passed to the gateway.
const SECRET_VARIABLES: &[&str] = &[
    token_store::SEALED_KEY_VARIABLE,
    "OP_SERVICE_ACCOUNT_TOKEN",
    "OP_CONNECT_TOKEN",
];

/// The keeper's folder: its socket and the sealed file.
pub fn folder(paths: &StatePaths) -> PathBuf {
    paths.root().join("keeper")
}

/// The keeper's socket.
pub fn socket(paths: &StatePaths) -> PathBuf {
    folder(paths).join("keeper.sock")
}

struct Users {
    keeper: nix::unistd::User,
    gateway: nix::unistd::User,
}

fn users() -> Option<Users> {
    if !nix::unistd::Uid::effective().is_root() {
        return None;
    }
    match (
        nix::unistd::User::from_name(KEEPER_USER).ok().flatten(),
        nix::unistd::User::from_name(GATEWAY_USER).ok().flatten(),
    ) {
        (Some(keeper), Some(gateway)) => Some(Users { keeper, gateway }),
        _ => None,
    }
}

/// Runs Hoista on a server until it's told to stop or a part of it stops.
pub async fn supervise(paths: &StatePaths) -> anyhow::Result<ExitCode> {
    // Fail early, here, on a store or key source Hoista doesn't have.
    token_store::token_store(
        std::env::var(token_store::TOKEN_STORE_VARIABLE)
            .ok()
            .as_deref(),
    )
    .map_err(|error| anyhow::anyhow!("{error}"))?;
    let source = token_store::key_source(
        std::env::var(token_store::KEY_SOURCE_VARIABLE)
            .ok()
            .as_deref(),
    )
    .map_err(|error| anyhow::anyhow!("{error}"))?;
    let key = source
        .sealing_key()
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    paths
        .prepare()
        .context("Could not prepare Hoista's state folder")?;
    let folder = folder(paths);
    std::fs::create_dir_all(&folder)?;
    let sealed = folder.join("vault.sealed");
    let socket = socket(paths);
    let _ = std::fs::remove_file(&socket);

    let users = users();
    match &users {
        Some(users) => {
            // Everything to the gateway; the keeper's folder and the sealed
            // file to the keeper alone. Others may pass through the state
            // folder to reach the socket, never list it.
            own_tree(paths.root(), &users.gateway, &folder)?;
            std::fs::set_permissions(paths.root(), std::fs::Permissions::from_mode(0o711))?;
            nix::unistd::chown(&folder, Some(users.keeper.uid), Some(users.keeper.gid))?;
            std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o711))?;
            if sealed.exists() {
                nix::unistd::chown(&sealed, Some(users.keeper.uid), Some(users.keeper.gid))?;
                std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o600))?;
            }
            eprintln!(
                "Hoista: the credential keeper runs as {KEEPER_USER}, the rest as {GATEWAY_USER}."
            );
        }
        None => eprintln!(
            "Hoista: no {GATEWAY_USER} and {KEEPER_USER} users (or not started as root), so all of Hoista runs as one user."
        ),
    }
    let gateway_uid = users
        .as_ref()
        .map_or_else(nix::unistd::Uid::effective, |users| users.gateway.uid);

    // The keeper: the key on its input, never in its arguments or environment.
    let exe = std::env::current_exe()?;
    let mut keeper = tokio::process::Command::new(&exe);
    keeper
        .arg("--credential-keeper")
        .arg("--state-dir")
        .arg(paths.root())
        .arg("--allow-uid")
        .arg(gateway_uid.as_raw().to_string())
        .env_clear()
        .env("HOME", &folder)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        // Signals from a terminal reach only the supervisor, which stops the rest in order.
        .process_group(0)
        .kill_on_drop(true);
    pass(&mut keeper, &["PATH"]);
    pass(&mut keeper, KEEPER_VARIABLES);
    if let Some(users) = &users {
        // Run as root, std also drops every supplementary group.
        keeper
            .uid(users.keeper.uid.as_raw())
            .gid(users.keeper.gid.as_raw());
    }
    let mut keeper = keeper
        .spawn()
        .context("Couldn't start the credential keeper")?;
    let mut keeper_input = keeper.stdin.take().context("The keeper has no input")?;
    keeper_input.write_all(key.expose().as_bytes()).await?;
    keeper_input.write_all(b"\n").await?;
    keeper_input.flush().await?;
    drop(key);
    drop(source);
    let deadline = tokio::time::Instant::now() + START_TIMEOUT;
    while !socket.exists() {
        if let Some(status) = keeper.try_wait()? {
            bail!("The credential keeper stopped while starting ({status})");
        }
        if tokio::time::Instant::now() > deadline {
            bail!("The credential keeper didn't start within 15 seconds");
        }
        tokio::time::sleep(START_POLL).await;
    }

    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        let mut gateway = gateway_command(&exe, paths, users.as_ref())
            .spawn()
            .context("Couldn't start Hoista")?;
        let pid = gateway.id();
        tokio::select! {
            status = gateway.wait() => {
                let status = status?;
                if status.success() && keeper.try_wait()?.is_none() {
                    // It asked to restart; start it again.
                    continue;
                }
                eprintln!("Hoista stopped ({status}).");
                drop(keeper_input);
                let _ = tokio::time::timeout(STOP_TIMEOUT, keeper.wait()).await;
                return Ok(ExitCode::FAILURE);
            }
            status = keeper.wait() => {
                eprintln!("Hoista's credential keeper stopped ({}); stopping Hoista.", status?);
                stop(pid, &mut gateway).await;
                return Ok(ExitCode::FAILURE);
            }
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
        // Told to stop: the gateway first, so it can finish its work, then the keeper.
        stop(pid, &mut gateway).await;
        drop(keeper_input);
        let _ = tokio::time::timeout(STOP_TIMEOUT, keeper.wait()).await;
        return Ok(ExitCode::SUCCESS);
    }
}

fn gateway_command(
    exe: &Path,
    paths: &StatePaths,
    users: Option<&Users>,
) -> tokio::process::Command {
    let mut gateway = tokio::process::Command::new(exe);
    gateway
        .arg("--state-dir")
        .arg(paths.root())
        .arg("--gateway")
        .stdin(std::process::Stdio::null())
        .process_group(0)
        .kill_on_drop(true);
    for name in SECRET_VARIABLES {
        gateway.env_remove(name);
    }
    if let Some(users) = users {
        gateway
            .env("HOME", paths.root())
            .env("USER", GATEWAY_USER)
            .uid(users.gateway.uid.as_raw())
            .gid(users.gateway.gid.as_raw());
    }
    gateway
}

async fn stop(pid: Option<u32>, child: &mut tokio::process::Child) {
    if let Some(pid) = pid.and_then(|pid| i32::try_from(pid).ok()) {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGTERM,
        );
    }
    if tokio::time::timeout(EXIT_TIMEOUT, child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
    }
}

fn pass(command: &mut tokio::process::Command, names: &[&str]) {
    for name in names {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
}

/// Gives everything under `root`, except `skip`, to `user`.
fn own_tree(root: &Path, user: &nix::unistd::User, skip: &Path) -> anyhow::Result<()> {
    nix::unistd::chown(root, Some(user.uid), Some(user.gid))?;
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        if path == skip {
            continue;
        }
        let kind = std::fs::symlink_metadata(&path)?.file_type();
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            own_tree(&path, user, skip)?;
        } else {
            nix::unistd::chown(&path, Some(user.uid), Some(user.gid))?;
        }
    }
    Ok(())
}

/// Being the keeper: `hoistd --credential-keeper --state-dir PATH --allow-uid N`,
/// with the sealing key on standard input. Runs until its input closes.
pub async fn run(root: PathBuf, allowed_uid: u32) -> anyhow::Result<()> {
    let paths = StatePaths::new(Some(root))?;
    let _ = hoista_server::infrastructure::tool_paths::initialize(&paths);
    let store = token_store::token_store(
        std::env::var(token_store::TOKEN_STORE_VARIABLE)
            .ok()
            .as_deref(),
    )
    .map_err(|error| anyhow::anyhow!("{error}"))?;
    store
        .check()
        .await
        .map_err(|error| anyhow::anyhow!("Hoista's token store isn't usable: {error}"))?;
    let mut key = String::new();
    std::io::stdin().read_line(&mut key)?;
    let key = hoista_server::domain::Secret::new(key.trim().to_owned());
    let vault = hoista_server::wiring::keeper_vault(&paths, key.expose())
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    drop(key);
    let keeper = Arc::new(
        hoista_server::infrastructure::keeper::Keeper::new(vault, Some(allowed_uid))
            .map_err(|error| anyhow::anyhow!("{error}"))?
            .with_store(Arc::from(store)),
    );
    let stop = tokio_util::sync::CancellationToken::new();
    {
        // Stops when the supervisor goes away (its end of our input closes).
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut rest = String::new();
            while std::io::stdin()
                .read_line(&mut rest)
                .is_ok_and(|read| read > 0)
            {
                rest.clear();
            }
            stop.cancel();
        });
    }
    keeper
        .serve(socket(&paths), stop)
        .await
        .context("The credential keeper's socket failed")
}
