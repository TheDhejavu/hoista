//! The Hoista daemon: builds the engine, serves it on this computer or a
//! server, and stops it in order on a signal or a restart request.

#![cfg_attr(windows, windows_subsystem = "windows")]

use anyhow::{Context, bail};
use hoista_server::{
    infrastructure::{paths::StatePaths, store::SqliteStore},
    server,
};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Daily engine log files kept.
const LOG_DAYS_KEPT: usize = 14;
/// The crates whose messages go into the engine logs.
const LOGGED_CRATES: &[&str] = &[
    "hoistd",
    "hoista_server",
    "hoista_app",
    "hoista_providers",
    "hoista_store",
    "hoista_system",
];
/// The port Hoista on a server listens on when `PORT` isn't set.
const DEFAULT_SERVER_PORT: u16 = 8080;

#[cfg(unix)]
mod keeper;

/// The `hoist` binary installed beside this daemon, with this daemon's state
/// folder when one was given, then `mcp`: what a coding tool should run.
fn mcp_command(root: Option<&std::path::Path>) -> Vec<String> {
    let name = if cfg!(windows) { "hoist.exe" } else { "hoist" };
    let Some(binary) = std::env::current_exe()
        .ok()
        .and_then(|exe| std::fs::canonicalize(exe).ok())
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
        .filter(|path| path.is_file())
        .and_then(|path| path.to_str().map(str::to_owned))
    else {
        return Vec::new();
    };
    let mut command = vec![binary];
    if let Some(root) = root.and_then(|root| std::path::absolute(root).ok())
        && let Some(root) = root.to_str()
    {
        command.extend(["--state-dir".to_owned(), root.to_owned()]);
    }
    command.push("mcp".to_owned());
    command
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The credential keeper, started by hoistd itself on a server.
    #[cfg(unix)]
    if let [
        "--credential-keeper",
        "--state-dir",
        path,
        "--allow-uid",
        uid,
    ] = args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        return keeper::run(
            PathBuf::from(path),
            uid.parse().context("--allow-uid takes a number")?,
        )
        .await;
    }
    // The part of a server's Hoista that its supervisor starts; it has no secrets.
    let gateway = args.last().is_some_and(|arg| arg == "--gateway");
    let args = if gateway {
        &args[..args.len() - 1]
    } else {
        &args[..]
    };
    let (root, supervised): (Option<PathBuf>, bool) = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => (None, false),
        ["--supervised"] => (None, true),
        ["--state-dir", path] => (Some(PathBuf::from(path)), false),
        ["--state-dir", path, "--supervised"] | ["--supervised", "--state-dir", path] => {
            (Some(PathBuf::from(path)), true)
        }
        ["--help" | "-h"] => {
            println!("Usage: hoistd [--state-dir PATH] [--supervised]");
            println!("       hoistd --new-sealed-key   (a key for HOISTA_SEALED_KEY on a server)");
            println!();
            println!("On a server, set HOISTA_PUBLIC_URL=https://your-host and HOISTA_SEALED_KEY;");
            println!("Hoista then listens on PORT (default 8080) for agents and its owner.");
            return Ok(());
        }
        ["--new-sealed-key"] => {
            println!(
                "{}",
                hoista_server::infrastructure::sealed_store::SealedStore::new_key()
            );
            return Ok(());
        }
        _ => bail!("Usage: hoistd [--state-dir PATH] [--supervised]"),
    };
    let mcp = mcp_command(root.as_deref());
    let paths = StatePaths::new(root)?;
    // On a server: HOISTA_PUBLIC_URL is this Hoista's https address.
    let public_url = std::env::var("HOISTA_PUBLIC_URL")
        .ok()
        .filter(|url| !url.trim().is_empty());
    #[cfg(unix)]
    if public_url.is_some() && !gateway {
        let code = keeper::supervise(&paths).await?;
        std::process::exit(if code == std::process::ExitCode::SUCCESS {
            0
        } else {
            1
        });
    }

    #[cfg(not(unix))]
    let _ = gateway;
    hoista_server::infrastructure::tool_paths::initialize(&paths)
        .context("Could not load deployment tool paths; run hoist tools configure to repair the configuration")?;
    paths
        .prepare()
        .context("Could not prepare Hoista's state folder")?;
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("hoistd")
        .filename_suffix("jsonl")
        .max_log_files(LOG_DAYS_KEPT)
        .build(paths.logs())
        .context("Could not open engine logs")?;
    let (writer, _log_guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(writer)
                .with_ansi(false),
        )
        .with(
            tracing_subscriber::filter::Targets::new().with_targets(
                LOGGED_CRATES
                    .iter()
                    .map(|target| (*target, tracing::Level::INFO)),
            ),
        )
        .try_init()
        .context("Could not initialize engine logs")?;
    // A signal or a fatal failure may cancel work; a restart the owner asks
    // for only stops new work coming in.
    let force_shutdown = CancellationToken::new();
    let shutdown = force_shutdown.child_token();
    let drain = CancellationToken::new();
    let lifecycle = std::sync::Arc::new(
        hoista_server::lifecycle::DaemonLifecycle::new(shutdown.clone(), supervised)
            .with_mcp_command(mcp),
    );
    let signal_token = force_shutdown.clone();
    let signals = tokio::spawn(async move {
        let result = termination_signal().await;
        signal_token.cancel();
        result
    });
    let store = SqliteStore::open(&paths)
        .await
        .context("Could not start hoistd")?;
    let engine = std::sync::Arc::new(match &public_url {
        // Started by the supervisor: provider tokens are with the credential keeper.
        #[cfg(unix)]
        Some(url) => hoista_server::wiring::kept_cloud_engine(
            store.clone(),
            &paths,
            url,
            std::sync::Arc::new(hoista_server::infrastructure::keeper::KeeperClient::new(
                keeper::socket(&paths),
            )),
        )?,
        #[cfg(not(unix))]
        Some(url) => {
            let key = std::env::var("HOISTA_SEALED_KEY")
                .context("Set HOISTA_SEALED_KEY to a new key from `hoistd --new-sealed-key`, kept as a secret variable at your provider")?;
            hoista_server::wiring::cloud_engine(store.clone(), &paths, url, &key)?
        }
        None => hoista_server::wiring::engine(store.clone(), &paths)?,
    });
    // A token store that can't be used stops Hoista here, with a plain message.
    engine
        .check_vault()
        .await
        .map_err(|error| anyhow::anyhow!("Hoista's token store isn't usable: {error}"))?;
    let worker_engine = engine.clone();
    let worker_shutdown = force_shutdown.clone();
    let worker_drain = drain.clone();
    let worker = tokio::spawn(async move {
        let result = worker_engine
            .run_deployments_until_idle(worker_shutdown.clone(), worker_drain.clone())
            .await;
        if result.is_err() || !worker_drain.is_cancelled() {
            worker_shutdown.cancel();
        }
        result
    });
    let monitor_engine = engine.clone();
    let monitor_shutdown = shutdown.clone();
    let monitor_force = force_shutdown.clone();
    let monitor = tokio::spawn(async move {
        let result = monitor_engine.run_monitor(monitor_shutdown.clone()).await;
        if result.is_err() || !monitor_shutdown.is_cancelled() {
            monitor_force.cancel();
        }
        result
    });
    // Webhooks for background agents: queued, signed and sent until shutdown.
    let webhooks = tokio::spawn(
        hoista_server::infrastructure::webhooks::Dispatcher::new(
            store.clone(),
            engine.vault.clone(),
        )
        .run(shutdown.clone()),
    );
    // Automatic backups, beside the state, until shutdown.
    let backups = tokio::spawn(engine.clone().run_backups(shutdown.clone()));
    // History kept for as long as the person chose; pruned daily.
    let retention = tokio::spawn(hoista_server::infrastructure::retention::run(
        store.clone(),
        shutdown.clone(),
    ));
    let logs_engine = engine.clone();
    let logs_shutdown = shutdown.clone();
    let logs = tokio::spawn(async move {
        logs_shutdown.cancelled().await;
        logs_engine.observe().stop_logs().await;
    });
    // The page where the person authorizes an agent, on this computer's
    // address, and on its private network address only when the person
    // turned that on in Settings → Agents.
    let network = engine
        .repository
        .settings()
        .await
        .is_ok_and(|settings| settings.network_page);
    let mut mcp_http = None;
    if let Some(url) = &public_url {
        // Set by the Hoista that created this server, which opens the claim
        // page with it; otherwise a new code is printed here.
        let preset = std::env::var("HOISTA_CLAIM_CODE").ok();
        let (cloud, claim) =
            hoista_server::cloud::Cloud::new(engine.clone(), paths.root(), url, preset.as_deref())
                .map_err(anyhow::Error::msg)?;
        let claimed = claim.is_none();
        if let Some(code) = claim {
            // Only the person who runs the server can read its log.
            // The code rides after `#` in the link, so it never reaches a server
            // log or a proxy. The line ends with the code: start.sh reads it.
            println!(
                "Hoista has no owner yet. Set it up at {url}/#code={code} or enter the claim code {code}"
            );
            tracing::info!(code = "cloud.unclaimed", %url);
        }
        let port: u16 = std::env::var("PORT")
            .ok()
            .and_then(|port| port.parse().ok())
            .unwrap_or(DEFAULT_SERVER_PORT);
        let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
            .await
            .with_context(|| format!("Could not listen on port {port}"))?;
        // The same owner API `hoist` uses on a computer, for computers the
        // owner signed in from the server's page. No restart control.
        let api = hoista_server::http::router_with_lifecycle(
            store.clone(),
            shutdown.clone(),
            Some(engine.clone()),
            None,
        )
        .merge(hoista_server::catalog::router(engine.clone()))
        .merge(hoista_server::backups::router(engine.clone()));
        // Receiving a move from the owner's computer; finished before shutdown.
        let (moves, move_work) = hoista_server::transfer::receive_router(engine.clone());
        let api = api.merge(moves);
        let project = hoista_server::cloud::router_with_api(cloud, api);
        let stop = shutdown.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, project)
                .with_graceful_shutdown(stop.cancelled_owned())
                .await;
            move_work.drain().await;
        });
        tracing::info!(code = "cloud.ready", port);
        // Where things stand, for whoever set it up and reads this log.
        let address = url.trim_end_matches('/');
        println!("Hoista answers at {address}.");
        println!("Agents connect at {address}/mcp.");
        if claimed {
            println!("It has an owner. To use it from a computer: hoist remote add NAME {address}");
        } else {
            println!(
                "To make it yours, on your computer run hoist remote add NAME {address} and enter the code above."
            );
        }
        println!("To check it from anywhere: hoist remote check {address}");
    } else {
        if let Err(error) =
            hoista_server::pages::serve(engine.clone(), &paths, network, shutdown.clone()).await
        {
            tracing::warn!(code = "pages.unavailable", %error);
        }
        // MCP over HTTP, for web agents through a tunnel or proxy the person runs;
        // follows its setting while Hoista runs.
        mcp_http = Some(tokio::spawn(hoista_server::link::serve(
            engine.clone(),
            paths.clone(),
            shutdown.clone(),
        )));
    }
    let result = server::serve_with_lifecycle(
        &paths,
        store.clone(),
        shutdown.clone(),
        engine,
        Some(lifecycle.clone()),
    )
    .await;
    shutdown.cancel();
    if result.is_err() || !lifecycle.restart_requested() {
        force_shutdown.cancel();
    }
    // MCP over HTTP finishes the agent requests under way.
    if let Some(task) = mcp_http {
        let _ = task.await;
    }
    // First drain every request source. Only then can an empty deployment queue
    // prove all accepted requests (including secret-bearing ones) have finished.
    let (monitor_result, logs_result, webhooks_result, backups_result, retention_result) =
        tokio::join!(monitor, logs, webhooks, backups, retention);
    if !monitor_result.as_ref().is_ok_and(|value| value.is_ok())
        || logs_result.is_err()
        || webhooks_result.is_err()
        || backups_result.is_err()
        || retention_result.is_err()
    {
        force_shutdown.cancel();
    }
    if lifecycle.restart_requested() {
        tracing::info!(code = "daemon.requests_drained");
    }
    drain.cancel();
    let worker_result = worker.await;
    signals.abort();
    let signal_result = signals.await;
    store.close().await;
    drop(store);

    logs_result.context("Log cleanup task panicked")?;
    tracing::info!(code = "daemon.stopped");
    match signal_result {
        Ok(result) => result.context("Could not observe shutdown signal")?,
        Err(error) if error.is_cancelled() => {}
        Err(error) => return Err(error.into()),
    }
    worker_result
        .context("Deployment worker panicked")?
        .context("Deployment worker failed")?;
    monitor_result
        .context("Usage monitor panicked")?
        .context("Usage monitor failed")?;
    result.context("Local service stopped with an error")?;
    if lifecycle.restart_requested() && !force_shutdown.is_cancelled() {
        tracing::info!(code = "daemon.queue_drained");
        if lifecycle.handoff() {
            tracing::info!(code = "daemon.restart_handoff");
        } else if gateway {
            // The supervisor starts it again.
            tracing::info!(code = "daemon.restart_supervisor");
        } else {
            spawn_replacement(&paths)?;
        }
    }
    Ok(())
}

fn spawn_replacement(paths: &StatePaths) -> anyhow::Result<()> {
    let binary = std::fs::canonicalize(std::env::current_exe()?)?;
    let mut command = std::process::Command::new(binary);
    command
        .arg("--state-dir")
        .arg(paths.root())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    let _child = command
        .spawn()
        .context("Could not start replacement daemon after draining work")?;
    Ok(())
}

async fn termination_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = term.recv() => Ok(()),
        }
    }

    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await
}
