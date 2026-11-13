//! `hoista-demo`: Hoista's real engine on example data. The first run fills
//! the state folder with Sam Rivera's projects; every run serves the engine
//! on that folder's socket, so the desktop app (started with `HOISTA_SOCKET`
//! pointed at it) works exactly as it does with `hoistd`. Providers answer
//! from the demo world and builds produce nothing, so nothing reaches the
//! internet and nothing is billed.
//!
//! Usage: `hoista-demo --state-dir PATH [--fresh]`. `--fresh` starts over.

mod answers;
mod build;
mod logs;
mod providers;
mod seed;
mod vault;
mod world;

use anyhow::{Context, bail};
use hoista_server::{
    application::{StateStore, engine::Engine, ports::catalog::CatalogRepository},
    domain::Profile,
    infrastructure::{paths::StatePaths, store::SqliteStore},
    server,
};
use std::{path::PathBuf, sync::Arc};
use tokio_util::sync::CancellationToken;

const USAGE: &str = "Usage: hoista-demo --state-dir PATH [--fresh]";

struct Options {
    state: PathBuf,
    fresh: bool,
}

fn options() -> anyhow::Result<Options> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut state = None;
    let mut fresh = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--state-dir" => state = args.next().map(PathBuf::from),
            "--fresh" => fresh = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            _ => bail!(USAGE),
        }
    }
    let Some(state) = state else { bail!(USAGE) };
    Ok(Options { state, fresh })
}

/// The demo engine on `store`: the real engine with the demo's providers,
/// builds and vault.
fn engine(store: &SqliteStore, paths: &StatePaths) -> anyhow::Result<Engine> {
    let state = paths.root();
    let http = hoista_server::infrastructure::providers::http::ProviderHttp::new()
        .context("Could not set up the provider catalogs")?;
    let real = hoista_server::infrastructure::providers::adapters(&http);
    Ok(Engine::new(
        Arc::new(store.clone()),
        Arc::new(vault::DemoVault::open(state)),
        Arc::new(build::DemoInspector::new(build::code_folder(state))),
        providers::all(real),
        Arc::new(build::DemoBuilder),
    )
    .with_owner_check(Arc::new(
        hoista_server::infrastructure::owner_check::SystemOwnerCheck,
    ))
    .with_backup_folder(Arc::new(
        hoista_server::infrastructure::backups::LocalBackups::new(paths),
    )))
}

/// What a coding tool runs to reach the demo: the `hoist` beside this
/// binary, aimed at the demo's state folder. Empty when there's no `hoist`.
fn mcp_command(state: &std::path::Path) -> Vec<String> {
    let name = if cfg!(windows) { "hoist.exe" } else { "hoist" };
    let hoist = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|folder| folder.join(name)))
        .filter(|path| path.is_file());
    let (Some(hoist), Ok(state)) = (hoist, std::path::absolute(state)) else {
        return Vec::new();
    };
    vec![
        hoist.to_string_lossy().into_owned(),
        "--state-dir".into(),
        state.to_string_lossy().into_owned(),
        "mcp".into(),
    ]
}

/// Two coding tools waiting for Sam to let them connect, as when they've just
/// been set up. Requests wait ten minutes, so each run asks again.
async fn ask_to_connect(store: &SqliteStore) -> anyhow::Result<()> {
    use hoista_server::application::engine::{EnrollmentRequest, EnrollmentStatus};
    let now = chrono::Utc::now();
    let waits = now + chrono::Duration::minutes(10);
    for (id, name, code) in [
        ("request-windsurf", "Windsurf", "KQ4M-7TXP"),
        ("request-zed", "Zed", "HV2N-9WRC"),
    ] {
        store
            .put_enrollment(&hoista_server::application::engine::StoredEnrollment {
                request: EnrollmentRequest {
                    id: id.into(),
                    name: name.into(),
                    transport: hoista_server::domain::model::AgentTransport::Stdio,
                    code: code.into(),
                    created_at: now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    expires_at: waits.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    status: EnrollmentStatus::Pending,
                    can_ask: Some(true),
                },
                channel: format!("{:064x}", code.len()),
                agent_id: None,
            })
            .await?;
    }
    Ok(())
}

/// Seeds an empty state folder.
///
/// Hoista's activity can't be edited, and every record written through the
/// store adds an entry stamped now. So the records are written into a scratch
/// database first, then copied into the real one beside the demo's own
/// timeline, spread over the weeks before.
async fn seed(paths: &StatePaths) -> anyhow::Result<bool> {
    let state = paths.root();
    let store = SqliteStore::open(paths)
        .await
        .context("Could not open the demo's state")?;
    let empty = store.projects(None).await?.items.is_empty();
    store.close().await;
    if !empty {
        return Ok(false);
    }
    let now = chrono::Utc::now();
    let scratch = StatePaths::new(Some(state.join(SCRATCH)))?;
    scratch.prepare()?;
    let written = SqliteStore::open(&scratch)
        .await
        .context("Could not open the scratch database")?;
    let profile = Profile::new("Sam Rivera".into(), "SR".into())
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    written.save_profile(&profile).await?;
    seed::seed(&written, &vault::DemoVault::open(state), state, now).await?;
    written.close().await;
    copy_records(paths, &scratch, now).await?;
    std::fs::remove_dir_all(scratch.root()).context("Could not remove the scratch database")?;
    println!("Seeded the demo with Sam Rivera's projects.");
    Ok(true)
}

/// Where the records are written before they're copied.
const SCRATCH: &str = "seeding";

/// Copies every record but the activity from `scratch` into the demo's
/// database, then writes the demo's timeline.
async fn copy_records(
    paths: &StatePaths,
    scratch: &StatePaths,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<()> {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(paths.database())
        .foreign_keys(false);
    let pool = sqlx::SqlitePool::connect_with(options)
        .await
        .context("Could not open the demo's database")?;
    let mut connection = pool.acquire().await?;
    sqlx::query("ATTACH DATABASE ? AS scratch")
        .bind(scratch.database().to_string_lossy().into_owned())
        .execute(&mut *connection)
        .await?;
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM scratch.sqlite_master WHERE type = 'table' \
         AND name NOT IN ('events', '_sqlx_migrations') AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_all(&mut *connection)
    .await?;
    for table in tables {
        sqlx::query(&format!(
            "INSERT OR REPLACE INTO main.\"{table}\" SELECT * FROM scratch.\"{table}\""
        ))
        .execute(&mut *connection)
        .await
        .with_context(|| format!("Could not copy {table}"))?;
    }
    for entry in seed::timeline(now) {
        sqlx::query(
            "INSERT INTO events (at, actor, project_id, kind, summary, detail) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(entry.at)
        .bind(entry.actor)
        .bind(entry.project)
        .bind(entry.kind)
        .bind(entry.summary)
        .bind(entry.detail)
        .execute(&mut *connection)
        .await?;
    }
    sqlx::query("DETACH DATABASE scratch")
        .execute(&mut *connection)
        .await?;
    drop(connection);
    pool.close().await;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Options { state, fresh } = options()?;
    if fresh && state.exists() {
        std::fs::remove_dir_all(&state).context("Could not clear the demo's state folder")?;
    }
    let paths = StatePaths::new(Some(state.clone()))?;
    paths
        .prepare()
        .context("Could not prepare the demo's state folder")?;
    let seeded = seed(&paths).await?;

    let store = SqliteStore::open(&paths)
        .await
        .context("Could not open the demo's state")?;
    let engine = Arc::new(engine(&store, &paths)?);
    ask_to_connect(&store).await?;
    if seeded {
        // A first backup, as Hoista keeps one a day.
        engine.data().save_backup().await?;
    }
    let shutdown = CancellationToken::new();
    let drain = CancellationToken::new();
    let lifecycle = Arc::new(
        hoista_server::lifecycle::DaemonLifecycle::new(shutdown.clone(), false)
            .with_mcp_command(mcp_command(paths.root())),
    );
    let worker = tokio::spawn({
        let engine = engine.clone();
        let (shutdown, drain) = (shutdown.clone(), drain.clone());
        async move { engine.run_deployments_until_idle(shutdown, drain).await }
    });
    let monitor = tokio::spawn({
        let engine = engine.clone();
        let shutdown = shutdown.clone();
        async move { engine.run_monitor(shutdown).await }
    });
    let stop = shutdown.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        stop.cancel();
    });
    println!(
        "Demo Hoista is running. Socket: {}",
        paths.socket().display()
    );
    println!("Stop it with Ctrl-C.");
    let served = server::serve_with_lifecycle(
        &paths,
        store.clone(),
        shutdown.clone(),
        engine,
        Some(lifecycle),
    )
    .await;
    shutdown.cancel();
    drain.cancel();
    let _ = tokio::join!(worker, monitor);
    store.close().await;
    served.context("The demo engine stopped with an error")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn seeds_an_empty_folder_once_with_the_whole_world() {
        let folder = tempfile::tempdir().expect("temporary folder");
        let paths = StatePaths::new(Some(folder.path().to_path_buf())).expect("paths");
        paths.prepare().expect("state folder");
        assert!(seed(&paths).await.expect("seeded"));
        assert!(!seed(&paths).await.expect("seen as seeded"));

        let store = SqliteStore::open(&paths).await.expect("store");
        let projects = store.projects(None).await.expect("projects").items;
        assert_eq!(projects.len(), world::PROJECTS.len());
        let accounts = store.accounts(None).await.expect("accounts").items;
        assert_eq!(accounts.len(), world::ACCOUNTS.len());
        let moodboard = store
            .deploys("moodboard", None)
            .await
            .expect("deploys")
            .items;
        assert_eq!(moodboard.len(), 5);
        assert!(store.plan("plan-moodboard").await.expect("plan").is_some());
        // Only the demo's own timeline: nothing stamped while seeding.
        let activity = store
            .events_before(None, None, None, None)
            .await
            .expect("activity");
        let now = chrono::Utc::now().to_rfc3339();
        assert!(
            activity
                .iter()
                .all(|event| event.at.as_str() < now.as_str())
        );
        store.close().await;
    }
}
