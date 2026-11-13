//! Builds a Worker fixture with a real Wrangler install from its lockfile; no
//! provider accounts needed. Ignored by default.

use hoista_server::{
    Provider,
    application::{deploys::planning::plan, ports::build::BuildService},
    domain::model::*,
    infrastructure::{build::LocalBuilder, inspection::detect},
};

#[tokio::test]
#[ignore = "requires Node 22+ and npm registry access; installs the checked-in fixture lockfile"]
async fn real_wrangler_bundles_scheduled_worker_without_public_assets() {
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cron-worker");
    let detected = detect(&source).expect("source inspection");
    assert_eq!(detected.framework, "worker");
    let account = Account {
        last_discovery: None,
        id: "fixture".into(),
        provider: Provider::Cloudflare,
        label: "unconnected fixture; no provider calls".into(),
        external_id: "unused".into(),
        status: AccountStatus::Connected,
        scopes: vec![],
        connected_at: "2026-11-02T00:00:00Z".into(),
        credential: None,
        in_1password: false,
    };
    let plan = plan(
        Source::Folder {
            path: source.to_string_lossy().into(),
        },
        "scheduled-fixture",
        detected,
        &[account],
        &providers(),
        None,
        None,
    )
    .expect("plan");
    assert_eq!(plan.resources[0].kind, ResourceKind::Job);
    assert_eq!(
        plan.resources[0].schedules,
        Some(vec!["*/5 * * * *".into()])
    );
    let state = tempfile::tempdir().unwrap();
    let builder = LocalBuilder::new(state.path());
    let id = uuid::Uuid::new_v4().to_string();
    let result = builder.build(&plan, &id, vec![]).await;
    // Cleanup runs even when assertions fail after the provider-free build attempt.
    builder.cleanup(&id).await.expect("workspace cleanup");
    let artifact = result.expect("real Wrangler build");
    assert!(
        artifact.assets.is_empty(),
        "server code cannot be shipped as public assets"
    );
    assert!(artifact.image.is_none());
    assert!(artifact.secrets.is_empty());
    let module = String::from_utf8(artifact.worker.expect("Worker module")).unwrap();
    assert!(module.contains("scheduled("));
    assert!(
        module.contains("hoista-scheduled-fixture:"),
        "local import was bundled"
    );
    assert!(!module.contains("from \"./message.js\""));
    assert!(!state.path().join("builds").join(id).exists());
}

#[tokio::test]
#[ignore = "requires Node 22+ and npm registry access; installs the checked-in fixture lockfile"]
async fn real_tanstack_ssr_separates_server_functions_from_public_assets() {
    assert_ssr_fixture("tanstack-ssr", "tanstack-start").await;
}

#[tokio::test]
#[ignore = "requires Node 22+ and npm registry access; installs the checked-in fixture lockfile"]
async fn real_next_ssr_adapts_next_build_and_runs_packaged_worker() {
    assert_ssr_fixture("next-ssr", "nextjs").await;
}

async fn assert_ssr_fixture(fixture: &str, framework: &str) {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(fixture);
    let detected = detect(&source).expect("source inspection");
    assert_eq!(detected.framework, framework);
    let account = Account {
        last_discovery: None,
        id: "fixture".into(),
        provider: Provider::Cloudflare,
        label: "offline provider fixture".into(),
        external_id: "unused".into(),
        status: AccountStatus::Connected,
        scopes: vec![],
        connected_at: "2026-11-02T00:00:00Z".into(),
        credential: None,
        in_1password: false,
    };
    let plan = plan(
        Source::Folder {
            path: source.to_string_lossy().into(),
        },
        "ssr-fixture",
        detected,
        &[account],
        &providers(),
        None,
        None,
    )
    .expect("SSR fixture setup");
    assert_eq!(plan.resources[0].kind, ResourceKind::Web);
    let state = tempfile::tempdir().expect("SSR fixture setup");
    let builder = LocalBuilder::new(state.path());
    let id = uuid::Uuid::new_v4().to_string();
    let result = builder.build(&plan, &id, vec![]).await;
    let artifact = result.expect("real SSR build");
    let workspace = state.path().join("builds").join(&id);
    let packaged = workspace.join("runtime-artifact");
    tokio::fs::create_dir(&packaged)
        .await
        .expect("SSR fixture setup");
    let entry = packaged.join(&artifact.worker_name);
    tokio::fs::write(&entry, artifact.worker.as_ref().expect("SSR fixture setup"))
        .await
        .expect("SSR fixture setup");
    for module in &artifact.worker_modules {
        let target = packaged.join(module.path.trim_start_matches('/'));
        tokio::fs::create_dir_all(target.parent().expect("SSR fixture setup"))
            .await
            .expect("SSR fixture setup");
        tokio::fs::write(target, &module.bytes)
            .await
            .expect("SSR fixture setup");
    }
    tokio::fs::write(
        workspace.join("runtime-check.mjs"),
        include_str!("worker_runtime.mjs"),
    )
    .await
    .expect("SSR fixture setup");
    let mut command = tokio::process::Command::new("node");
    command
        .arg("runtime-check.mjs")
        .arg(std::path::Path::new("runtime-artifact").join(&artifact.worker_name))
        .current_dir(&workspace)
        .env_clear()
        .kill_on_drop(true);
    for key in ["PATH", "HOME", "SYSTEMROOT", "TEMP", "TMP", "USERPROFILE"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }

    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().expect("local Workers runtime");
    #[cfg(unix)]
    let group = nix::unistd::Pid::from_raw(
        i32::try_from(child.id().expect("SSR fixture setup")).expect("SSR fixture setup"),
    );
    let status = tokio::time::timeout(std::time::Duration::from_secs(60), child.wait()).await;
    #[cfg(unix)]
    let _ = nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGKILL);
    if status.is_err() {
        child.kill().await.expect("SSR fixture setup");
    }
    builder.cleanup(&id).await.expect("workspace cleanup");
    assert!(
        status
            .expect("runtime deadline")
            .expect("runtime exit")
            .success(),
        "packaged SSR Worker must render its server function result"
    );
    assert!(
        artifact
            .assets
            .iter()
            .any(|a| a.path == "/fixture.txt" && a.bytes.starts_with(b"hoista-public-fixture"))
    );
    assert!(
        artifact.assets.iter().any(|a| a.path.ends_with(".js")),
        "hydration assets were retained"
    );
    assert!(
        artifact
            .assets
            .iter()
            .all(|a| !String::from_utf8_lossy(&a.bytes).contains("hoista-server-only-fixture")),
        "server function code leaked into public assets"
    );
    let module = artifact.worker.expect("SSR Worker entry");
    assert!(
        String::from_utf8_lossy(&module).contains("hoista-server-only-fixture")
            || artifact
                .worker_modules
                .iter()
                .any(|a| String::from_utf8_lossy(&a.bytes).contains("hoista-server-only-fixture")),
        "server function was retained for runtime execution"
    );
    assert!(artifact.image.is_none());
    assert!(artifact.secrets.is_empty());
    assert!(
        artifact
            .worker_modules
            .iter()
            .all(|module| !module.path.ends_with(".map"))
    );
}

#[tokio::test]
#[ignore = "requires Node 22+ and npm registry access; runs locked Vite and Supabase SPA builds without accounts"]
async fn real_vite_and_supabase_spas_build_public_assets_without_backend_migration() {
    use hoista_server::domain::Secret;
    for fixture in ["vite-spa", "supabase-spa"] {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/apps")
            .join(fixture);
        let detected = detect(&source).unwrap();
        let account = Account {
            last_discovery: None,
            id: "fixture".into(),
            provider: Provider::Cloudflare,
            label: "no provider calls".into(),
            external_id: "unused".into(),
            status: AccountStatus::Connected,
            scopes: vec![],
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        };
        let plan = plan(
            Source::Folder {
                path: source.to_string_lossy().into(),
            },
            "spa-fixture",
            detected,
            &[account],
            &providers(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(plan.resources.len(), 1);
        assert!(plan.prisma_migration.is_none());
        let values = if fixture == "vite-spa" {
            vec![(
                "VITE_PUBLIC_LABEL".into(),
                Secret::new("hoista-public-label-fixture".into()),
            )]
        } else {
            vec![
                (
                    "VITE_SUPABASE_URL".into(),
                    Secret::new("https://hoista-fixture.supabase.co".into()),
                ),
                (
                    "VITE_SUPABASE_ANON_KEY".into(),
                    Secret::new("sb_publishable_hoista_fixture_not_real_0123456789".into()),
                ),
            ]
        };
        let state = tempfile::tempdir().unwrap();
        let builder = LocalBuilder::new(state.path());
        let id = uuid::Uuid::new_v4().to_string();
        let result = builder.build(&plan, &id, values).await;
        builder.cleanup(&id).await.unwrap();
        let artifact = result.expect("locked real SPA build");
        assert!(artifact.worker.is_none());
        assert!(artifact.image.is_none());
        assert!(artifact.secrets.is_empty());
        assert!(
            artifact
                .assets
                .iter()
                .any(|asset| asset.path == "/index.html")
        );
        let marker = if fixture == "vite-spa" {
            "hoista-public-label-fixture"
        } else {
            "https://hoista-fixture.supabase.co"
        };
        assert!(
            artifact
                .assets
                .iter()
                .any(|asset| asset.path.ends_with(".js")
                    && String::from_utf8_lossy(&asset.bytes).contains(marker))
        );
        assert!(
            artifact
                .assets
                .iter()
                .all(|asset| !asset.path.ends_with(".map")
                    && !asset.path.contains("node_modules")
                    && !asset.path.ends_with("package-lock.json"))
        );
    }
}

#[tokio::test]
#[ignore = "requires Node 22+ and npm registry access; prepares locked Express/Prisma tooling without a database"]
async fn real_express_prisma_context_prepares_locked_migration_tooling() {
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/apps/express-prisma");
    let accounts: Vec<_> = [Provider::Fly, Provider::Neon]
        .into_iter()
        .map(|provider| Account {
            last_discovery: None,
            id: provider.slug().into(),
            provider,
            label: "no provider calls".into(),
            external_id: "unused".into(),
            status: AccountStatus::Connected,
            scopes: vec![],
            connected_at: "2026-11-02T00:00:00Z".into(),
            credential: None,
            in_1password: false,
        })
        .collect();
    let plan = plan(
        Source::Folder {
            path: source.to_string_lossy().into(),
        },
        "express-fixture",
        detect(&source).unwrap(),
        &accounts,
        &providers(),
        None,
        None,
    )
    .unwrap();
    let state = tempfile::tempdir().unwrap();
    let builder = LocalBuilder::new(state.path());
    let id = uuid::Uuid::new_v4().to_string();
    let result = builder.build(&plan, &id, vec![]).await;
    let verified = result.as_ref().map(|artifact| {
        let context = std::path::Path::new(artifact.image.as_deref().unwrap());
        assert!(context.join("node_modules/prisma/build/index.js").is_file());
        assert!(
            context
                .join("prisma/migrations/20260101_initial/migration.sql")
                .is_file()
        );
        assert!(
            std::fs::read_to_string(context.join(".dockerignore"))
                .unwrap()
                .contains("node_modules")
        );
        assert!(artifact.provenance.is_some());
        assert!(artifact.assets.is_empty() && artifact.worker.is_none());
    });
    builder.cleanup(&id).await.unwrap();
    verified.expect("locked Express/Prisma context preparation");
}

/// Every provider Hoista connects to, as plans are made against them.
fn providers() -> hoista_server::application::deploys::planning::Providers {
    hoista_server::application::deploys::planning::Providers::new(
        hoista_server::infrastructure::providers::provider_capabilities(),
    )
}
