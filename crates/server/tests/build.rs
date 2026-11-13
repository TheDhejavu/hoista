//! Build tests on real project files: secrets blocked before upload, and source
//! snapshots. Makes no provider calls.

use hoista_server::{
    application::ports::build::BuildService,
    domain::{Secret, model::*},
    infrastructure::build::LocalBuilder,
};

fn plan(path: &std::path::Path) -> Plan {
    Plan {
        revision: 0,
        generated_secret_names: vec![],
        prisma_migration: None,
        id: "plan".into(),
        source: Source::Folder {
            path: path.to_string_lossy().into(),
        },
        framework: "static".into(),
        package_manager: None,
        build_command: vec![],
        output_directory: ".".into(),
        env_names: vec![],
        resources: vec![],
        warnings: vec![],
    }
}

#[tokio::test]
async fn public_server_variables_and_embedded_values_block_before_upload() {
    let source = tempfile::tempdir().expect("source");
    let state = tempfile::tempdir().expect("state");
    let builder = LocalBuilder::new(state.path());
    let plan = plan(source.path());
    std::fs::write(
        source.path().join(".env"),
        "VITE_PRIVATE_SECRET=opaque-private-value",
    )
    .expect("environment");
    assert!(builder.secrets(&plan).await.is_err());
    std::fs::write(
        source.path().join(".env"),
        "SERVER_KEY=opaque-private-value",
    )
    .expect("environment");
    std::fs::write(
        source.path().join("index.html"),
        "leaked: opaque-private-value",
    )
    .expect("asset");
    let values = builder.secrets(&plan).await.expect("private environment");
    let id = uuid::Uuid::new_v4().to_string();
    let result = builder.build(&plan, &id, values).await;
    assert!(result.is_err());
    assert!(
        !result
            .err()
            .expect("error")
            .to_string()
            .contains("opaque-private-value")
    );
    builder.cleanup(&id).await.expect("cleanup");
}

#[tokio::test]
async fn snapshots_exclude_env_files_and_reject_output_escape() {
    let source = tempfile::tempdir().expect("source");
    let state = tempfile::tempdir().expect("state");
    let builder = LocalBuilder::new(state.path());
    let mut plan = plan(source.path());
    std::fs::write(
        source.path().join(".env"),
        "SERVER_KEY=opaque-private-value",
    )
    .expect("environment");
    std::fs::write(source.path().join("index.html"), "safe public page").expect("asset");
    for generated in [
        ".next",
        ".open-next",
        ".svelte-kit",
        ".wrangler",
        ".hoista-worker",
    ] {
        std::fs::create_dir(source.path().join(generated)).unwrap();
        std::fs::write(
            source.path().join(generated).join("worker.js"),
            "stale generated code",
        )
        .unwrap();
    }
    let id = uuid::Uuid::new_v4().to_string();
    let artifact = builder
        .build(
            &plan,
            &id,
            vec![(
                "SERVER_KEY".into(),
                Secret::new("opaque-private-value".into()),
            )],
        )
        .await
        .expect("artifact");
    assert_eq!(artifact.assets.len(), 1);
    assert!(!state.path().join("builds").join(&id).join(".env").exists());
    for generated in [
        ".next",
        ".open-next",
        ".svelte-kit",
        ".wrangler",
        ".hoista-worker",
    ] {
        assert!(
            !state
                .path()
                .join("builds")
                .join(&id)
                .join(generated)
                .exists()
        );
    }
    builder.cleanup(&id).await.expect("cleanup");
    plan.output_directory = "../outside".into();
    assert!(
        builder
            .build(&plan, &uuid::Uuid::new_v4().to_string(), vec![])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn github_sources_require_an_exact_commit_and_safe_repository() {
    use hoista_server::infrastructure::source::resolve;
    for (repository, commit) in [
        ("owner/repo", "main"),
        ("../repo", "0123456789012345678901234567890123456789"),
        (
            "https://evil.example/repo",
            "0123456789012345678901234567890123456789",
        ),
    ] {
        assert!(
            resolve(&Source::Github {
                repository: repository.into(),
                commit: commit.into()
            })
            .await
            .is_err()
        );
    }
}

#[tokio::test]
async fn private_worker_files_are_not_public_and_hidden_registry_credentials_block_context_upload()
{
    let source = tempfile::tempdir().expect("source");
    let state = tempfile::tempdir().expect("state");
    let builder = LocalBuilder::new(state.path());
    let mut plan = plan(source.path());
    std::fs::write(source.path().join("index.html"), "public").expect("asset");
    std::fs::write(
        source.path().join("_worker.js"),
        "private server implementation",
    )
    .expect("worker");
    std::fs::write(source.path().join("_routes.json"), "{}").expect("routes");
    let id = uuid::Uuid::new_v4().to_string();
    let artifact = builder.build(&plan, &id, vec![]).await.expect("build");
    assert_eq!(artifact.assets.len(), 1);
    assert_eq!(artifact.assets[0].path, "/index.html");
    builder.cleanup(&id).await.expect("cleanup");
    std::fs::write(
        source.path().join("Dockerfile"),
        "FROM scratch\nCOPY . /app\n",
    )
    .expect("docker");
    std::fs::write(
        source.path().join(".npmrc"),
        "//registry.example/:_authToken=hidden-registry-password",
    )
    .expect("registry config");
    plan.resources.push(ResourcePlan {
        schedules: None,
        name: "project".into(),
        kind: ResourceKind::Service,
        provider: hoista_server::Provider::Fly,
        account_id: "a".into(),
        provider_ref: None,
        region: None,
        depends_on: vec![],
    });
    let id = uuid::Uuid::new_v4().to_string();
    let error = builder
        .build(&plan, &id, vec![])
        .await
        .err()
        .expect("hidden credential rejected");
    assert!(!error.to_string().contains("hidden-registry-password"));
    builder.cleanup(&id).await.expect("cleanup");
}

#[tokio::test]
async fn git_staged_secrets_are_blocked_even_when_worktree_is_cleaned_and_helpers_are_disabled() {
    fn git(root: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", root.join("empty-config"))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }
    let source = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let builder = LocalBuilder::new(state.path());
    let plan = plan(source.path());
    git(source.path(), &["init", "--quiet"]);
    std::fs::write(source.path().join("index.html"), "safe").unwrap();
    let secret = "sk_test_abcdefghijklmnopqrstuvwxyz";
    std::fs::write(source.path().join("server.txt"), secret).unwrap();
    git(source.path(), &["add", "."]);
    std::fs::write(
        source.path().join("server.txt"),
        "removed from working tree",
    )
    .unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let error = builder.build(&plan, &id, vec![]).await.err().unwrap();
    assert_eq!(error.code(), "deploy.secret_leak");
    assert!(!error.to_string().contains(secret));
    assert!(!state.path().join("builds").join(&id).exists());
    // A cleaned index is deployable, including in a repository without a first commit.
    git(source.path(), &["add", "."]);
    let id = uuid::Uuid::new_v4().to_string();
    builder.build(&plan, &id, vec![]).await.unwrap();
    builder.cleanup(&id).await.unwrap();
    git(
        source.path(),
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "safe",
        ],
    );
    // Configured external diff/textconv helpers would fail if invoked.
    git(
        source.path(),
        &["config", "diff.external", "hoista-no-such-helper"],
    );
    git(
        source.path(),
        &["config", "diff.fixture.textconv", "hoista-no-such-helper"],
    );
    git(
        source.path(),
        &["config", "core.fsmonitor", "hoista-no-such-helper"],
    );
    std::fs::write(
        source.path().join(".gitattributes"),
        "server.txt diff=fixture filter=fixture\n",
    )
    .unwrap();
    git(
        source.path(),
        &["config", "filter.fixture.clean", "hoista-no-such-helper"],
    );
    git(
        source.path(),
        &["config", "filter.fixture.process", "hoista-no-such-helper"],
    );
    git(
        source.path(),
        &["config", "filter.fixture.required", "true"],
    );
    std::fs::write(source.path().join("server.txt"), "safe changed source").unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    builder.build(&plan, &id, vec![]).await.unwrap();
    builder.cleanup(&id).await.unwrap();
}

#[tokio::test]
async fn source_gate_covers_hidden_untracked_binary_files_and_allows_removed_credentials() {
    let source = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let builder = LocalBuilder::new(state.path());
    let plan = plan(source.path());
    std::fs::write(source.path().join("index.html"), "safe page").unwrap();
    std::fs::create_dir(source.path().join(".server")).unwrap();
    std::fs::write(
        source.path().join(".server/private.bin"),
        b"\0opaque-server-value\xff",
    )
    .unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let error = builder
        .build(
            &plan,
            &id,
            vec![(
                "SERVER_KEY".into(),
                Secret::new("opaque-server-value".into()),
            )],
        )
        .await
        .err()
        .unwrap();
    assert_eq!(error.code(), "deploy.secret_leak");
    assert!(!error.to_string().contains("opaque-server-value"));
    assert!(
        !state
            .path()
            .join("builds")
            .join(&id)
            .join(".server/private.bin")
            .exists()
    );
    builder.cleanup(&id).await.unwrap();
    std::fs::remove_file(source.path().join(".server/private.bin")).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    builder.build(&plan, &id, vec![]).await.unwrap();
    builder.cleanup(&id).await.unwrap();
}

#[tokio::test]
async fn git_inspection_scopes_monorepo_changes_to_the_selected_project() {
    let repo = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let project = repo.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join("index.html"), "safe project").unwrap();
    std::fs::write(
        repo.path().join("outside.txt"),
        "sk_test_abcdefghijklmnopqrstuvwxyz",
    )
    .unwrap();
    for args in [vec!["init", "--quiet"], vec!["add", "."]] {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(repo.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", repo.path().join("empty-config"))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }
    let builder = LocalBuilder::new(state.path());
    let id = uuid::Uuid::new_v4().to_string();
    let artifact = builder.build(&plan(&project), &id, vec![]).await.unwrap();
    assert_eq!(artifact.assets.len(), 1);
    assert_eq!(artifact.assets[0].path, "/index.html");
    builder.cleanup(&id).await.unwrap();
}

#[tokio::test]
async fn source_provenance_distinguishes_clean_dirty_and_exact_copied_bytes() {
    fn git(root: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", root.join("empty-config"))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }

    async fn identity(
        builder: &LocalBuilder,
        plan: &Plan,
    ) -> hoista_server::domain::provenance::SourceProvenance {
        let id = uuid::Uuid::new_v4().to_string();
        let result = builder
            .build(plan, &id, vec![])
            .await
            .unwrap()
            .provenance
            .unwrap();
        builder.cleanup(&id).await.unwrap();
        result
    }
    let source = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let builder = LocalBuilder::new(state.path());
    let plan = plan(source.path());
    std::fs::write(source.path().join("index.html"), "first version").unwrap();
    let folder = identity(&builder, &plan).await;
    assert!(folder.git_commit.is_none());
    assert!(folder.git_dirty.is_none());
    assert_eq!(folder.snapshot_files, 1);
    assert_eq!(folder.snapshot_sha256.len(), 64);
    assert_eq!(folder.snapshot_format, "hoista-source-v1");
    assert_eq!(
        identity(&builder, &plan).await.snapshot_sha256,
        folder.snapshot_sha256
    );
    git(source.path(), &["init", "--quiet"]);
    let unborn = identity(&builder, &plan).await;
    assert!(unborn.git_commit.is_none());
    assert_eq!(unborn.git_dirty, Some(true));
    assert_eq!(unborn.snapshot_sha256, folder.snapshot_sha256);
    git(source.path(), &["add", "."]);
    git(
        source.path(),
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "source identity",
        ],
    );
    let clean = identity(&builder, &plan).await;
    assert_eq!(clean.git_dirty, Some(false));
    assert_eq!(clean.git_commit.as_ref().unwrap().len(), 40);
    std::fs::write(source.path().join("index.html"), "second version").unwrap();
    let dirty = identity(&builder, &plan).await;
    assert_eq!(dirty.git_commit, clean.git_commit);
    assert_eq!(dirty.git_dirty, Some(true));
    assert_ne!(dirty.snapshot_sha256, clean.snapshot_sha256);
    std::fs::write(
        source.path().join(".env"),
        "PRIVATE_VALUE=not-resource-of-source\n",
    )
    .unwrap();
    assert_eq!(
        identity(&builder, &plan).await.snapshot_sha256,
        dirty.snapshot_sha256
    );
    std::fs::rename(
        source.path().join("index.html"),
        source.path().join("renamed.html"),
    )
    .unwrap();
    assert_ne!(
        identity(&builder, &plan).await.snapshot_sha256,
        dirty.snapshot_sha256
    );
}

#[cfg(unix)]
#[tokio::test]
async fn source_snapshot_preserves_executable_files_and_hashes_the_executable_flag() {
    use std::os::unix::fs::PermissionsExt;
    let source = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let builder = LocalBuilder::new(state.path());
    let plan = plan(source.path());
    let script = source.path().join("build.sh");
    std::fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let executable = builder
        .build(&plan, &id, vec![])
        .await
        .unwrap()
        .provenance
        .unwrap();
    assert_eq!(
        std::fs::metadata(state.path().join("builds").join(&id).join("build.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    builder.cleanup(&id).await.unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o644)).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let regular = builder
        .build(&plan, &id, vec![])
        .await
        .unwrap()
        .provenance
        .unwrap();
    assert_ne!(executable.snapshot_sha256, regular.snapshot_sha256);
    builder.cleanup(&id).await.unwrap();
}

#[derive(Default)]
struct SnapshotProgress {
    value: Option<hoista_server::domain::provenance::SourceProvenance>,
    reject: bool,
}

#[async_trait::async_trait]
impl hoista_server::application::ports::build::BuildProgress for SnapshotProgress {
    async fn source_snapshot(
        &mut self,
        value: &hoista_server::domain::provenance::SourceProvenance,
    ) -> Result<(), hoista_server::application::ports::catalog::EngineError> {
        self.value = Some(value.clone());
        if self.reject {
            Err(hoista_server::application::ports::catalog::EngineError::Store)
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn completed_snapshot_is_reported_before_build_failure_and_observer_failure_stops_execution()
{
    let source = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("index.html"), "safe source").unwrap();
    let builder = LocalBuilder::new(state.path());
    let mut plan = plan(source.path());
    plan.build_command = vec!["unsupported-build-command".into()];
    let mut progress = SnapshotProgress::default();
    let id = uuid::Uuid::new_v4().to_string();
    let error = builder
        .build_with_progress(&plan, &id, vec![], &mut progress)
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("Build commands must"));
    let snapshot = progress.value.unwrap();
    assert_eq!(snapshot.snapshot_files, 1);
    assert_eq!(snapshot.snapshot_sha256.len(), 64);
    builder.cleanup(&id).await.unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let mut progress = SnapshotProgress {
        reject: true,
        ..Default::default()
    };
    let error = builder
        .build_with_progress(&plan, &id, vec![], &mut progress)
        .await
        .err()
        .unwrap();
    assert!(matches!(
        error,
        hoista_server::application::ports::catalog::EngineError::Store
    ));
    assert_eq!(progress.value, Some(snapshot));
    builder.cleanup(&id).await.unwrap();
    std::fs::write(
        source.path().join("index.html"),
        "sk_live_aaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let mut progress = SnapshotProgress::default();
    assert!(
        builder
            .build_with_progress(&plan, &id, vec![], &mut progress)
            .await
            .is_err()
    );
    assert!(
        progress.value.is_none(),
        "a rejected partial snapshot must not be reported as completed"
    );
    builder.cleanup(&id).await.unwrap();
}

#[tokio::test]
async fn environment_overrides_replace_old_public_inputs_without_editing_source_files() {
    let source = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let old = "VITE_ANALYTICS_KEY=sk_live_aaaaaaaaaaaaaaaaaaaaaaaa\nSERVER_KEY=old-private\n";
    std::fs::write(source.path().join(".env"), old).unwrap();
    let builder = LocalBuilder::new(state.path());
    let plan = plan(source.path());
    assert!(builder.secrets(&plan).await.is_err());
    let values = builder
        .secrets_with_overrides(
            &plan,
            vec![
                (
                    "VITE_ANALYTICS_KEY".into(),
                    Secret::new("public-analytics".into()),
                ),
                (
                    "SERVER_KEY".into(),
                    Secret::new("replacement-private".into()),
                ),
            ],
        )
        .await
        .unwrap();
    assert_eq!(
        values
            .iter()
            .find(|(name, _)| name == "SERVER_KEY")
            .unwrap()
            .1
            .expose(),
        "replacement-private"
    );
    assert_eq!(
        std::fs::read_to_string(source.path().join(".env")).unwrap(),
        old
    );
    assert!(
        builder
            .secrets_with_overrides(
                &plan,
                vec![(
                    "VITE_ANALYTICS_KEY".into(),
                    Secret::new("sk_live_bbbbbbbbbbbbbbbbbbbbbbbb".into())
                )]
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn early_observation_distinguishes_folder_unborn_git_remote_and_unreadable_source() {
    use hoista_server::{
        application::ports::build::SourceInspector, domain::provenance::SourceObservation,
        infrastructure::inspection::LocalInspector,
    };
    let source = tempfile::tempdir().unwrap();
    let input = Source::Folder {
        path: source.path().to_string_lossy().into(),
    };
    assert_eq!(
        LocalInspector.observe_source(&input).await.unwrap(),
        SourceObservation::LocalFolder
    );
    assert!(
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(source.path())
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(
        LocalInspector.observe_source(&input).await.unwrap(),
        SourceObservation::LocalGit {
            commit: None,
            dirty: false
        }
    );
    std::fs::write(
        source.path().join("secret-name.txt"),
        "private-content-marker",
    )
    .unwrap();
    let value = LocalInspector.observe_source(&input).await.unwrap();
    assert_eq!(
        value,
        SourceObservation::LocalGit {
            commit: None,
            dirty: true
        }
    );
    let json = serde_json::to_string(&value).unwrap();
    assert!(!json.contains("secret-name"));
    assert!(!json.contains("private-content-marker"));
    assert_eq!(
        LocalInspector
            .observe_source(&Source::Github {
                repository: "no-network/needed".into(),
                commit: "a".repeat(40)
            })
            .await
            .unwrap(),
        SourceObservation::RemoteSource
    );
    source.close().unwrap();
    assert!(LocalInspector.observe_source(&input).await.is_err());
}
