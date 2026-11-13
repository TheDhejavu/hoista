//! The sample projects through detection, planning and leak checks, without
//! provider accounts.

use hoista_server::{
    Provider,
    application::{deploys::planning, ports::build::BuildService},
    domain::{leaks::LeakScanner, model::*},
    infrastructure::{build::LocalBuilder, inspection::detect},
};
use std::path::PathBuf;
fn accounts() -> Vec<Account> {
    [Provider::Cloudflare, Provider::Fly, Provider::Neon]
        .into_iter()
        .map(|provider| Account {
            id: provider.slug().into(),
            provider,
            label: "local fixture".into(),
            external_id: "fixture".into(),
            scopes: vec![],
            status: AccountStatus::Connected,
            connected_at: "2026-11-02T00:00:00Z".into(),
            last_discovery: None,
            credential: None,
            in_1password: false,
        })
        .collect()
}

fn fixture(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(path)
}

#[tokio::test]
async fn required_samples_detect_plan_and_gate_leaks_without_provider_access() {
    let state = tempfile::tempdir().unwrap();
    let builder = LocalBuilder::new(state.path());
    let scanner = LeakScanner::new().unwrap();
    for (path, framework, provider, database, leaked) in [
        ("apps/vite-spa", "vite", Provider::Cloudflare, false, false),
        (
            "tanstack-ssr",
            "tanstack-start",
            Provider::Cloudflare,
            false,
            false,
        ),
        ("next-ssr", "nextjs", Provider::Cloudflare, false, false),
        ("apps/express-prisma", "node", Provider::Fly, true, false),
        ("apps/fastapi", "python", Provider::Fly, false, false),
        (
            "apps/supabase-spa",
            "vite",
            Provider::Cloudflare,
            false,
            false,
        ),
        (
            "apps/leaked-key",
            "static",
            Provider::Cloudflare,
            false,
            true,
        ),
    ] {
        let root = fixture(path);
        let detected = detect(&root).unwrap_or_else(|error| panic!("{path}: {error}"));
        assert_eq!(detected.framework, framework, "{path}");
        assert_eq!(detected.postgres, database, "{path}");
        if path == "apps/fastapi" {
            assert_eq!(detected.env_names, ["APP_LABEL"]);
        }
        if path == "apps/express-prisma" {
            assert!(detected.prisma_migration.is_some());
            assert!(detected.env_names.contains(&"PORT".into()));
        }
        if path == "apps/supabase-spa" {
            assert_eq!(detected.shape, "supabase");
            assert_eq!(
                detected.env_names,
                ["VITE_SUPABASE_ANON_KEY", "VITE_SUPABASE_URL"]
            );
            assert!(
                detected
                    .warnings
                    .iter()
                    .any(|warning| warning.contains("existing Supabase"))
            );
        }
        let source = Source::Folder {
            path: root.to_string_lossy().into_owned(),
        };
        let plan = planning::plan(
            source,
            "sample",
            detected,
            &accounts(),
            &providers(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(plan.resources.last().unwrap().provider, provider, "{path}");
        assert_eq!(plan.resources.len(), if database { 2 } else { 1 });
        assert_eq!(
            planning::release_order(&plan.resources, &providers()).unwrap(),
            (0..plan.resources.len()).collect::<Vec<_>>()
        );
        if database {
            assert_eq!(plan.resources[0].provider, Provider::Neon);
            assert_eq!(plan.resources[1].depends_on, ["sample-db"]);
        }
        let secrets = builder.secrets(&plan).await.unwrap();
        assert!(secrets.is_empty());
        let mut findings = Vec::new();
        for entry in walkdir::WalkDir::new(&root)
            .into_iter()
            .filter_entry(|entry| {
                !["node_modules", "dist", ".next", ".open-next", ".git"]
                    .contains(&entry.file_name().to_string_lossy().as_ref())
            })
        {
            let entry = entry.unwrap();
            if entry.file_type().is_file() {
                let location = entry.path().strip_prefix(&root).unwrap().to_string_lossy();
                findings.extend(scanner.scan(
                    &location,
                    &std::fs::read(entry.path()).unwrap(),
                    &[],
                ));
            }
        }
        assert_eq!(!findings.is_empty(), leaked, "{path}: {findings:?}");
        if provider == Provider::Fly && !database {
            let id = uuid::Uuid::new_v4().to_string();
            let built = builder.build(&plan, &id, vec![]).await;
            let inspected = built.as_ref().map(|artifact| {
                let context =
                    std::path::Path::new(artifact.image.as_deref().expect("Fly build context"));
                assert!(context.join("Dockerfile").is_file());
                assert!(
                    context
                        .join(if database {
                            "prisma/migrations/20260101_initial/migration.sql"
                        } else {
                            "main.py"
                        })
                        .is_file()
                );
                assert!(artifact.assets.is_empty());
                assert!(artifact.worker.is_none());
                assert!(artifact.provenance.is_some());
            });
            builder.cleanup(&id).await.unwrap();
            inspected.unwrap();
        }
        if leaked {
            let id = uuid::Uuid::new_v4().to_string();
            let error = builder
                .build(&plan, &id, secrets)
                .await
                .err()
                .expect("leaked source must fail the real build gate");
            assert_eq!(error.code(), "deploy.secret_leak");
            assert!(!error.to_string().contains("HoistaFixtureNeverARealKey"));
            builder.cleanup(&id).await.unwrap();
        }
    }
}

/// Every provider Hoista connects to, as plans are made against them.
fn providers() -> hoista_server::application::deploys::planning::Providers {
    hoista_server::application::deploys::planning::Providers::new(
        hoista_server::infrastructure::providers::provider_capabilities(),
    )
}
