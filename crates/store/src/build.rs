//! Building a project on this computer: a copy of its source is taken and
//! checked for secrets, its build runs in the sandbox, and everything that
//! would be published is checked again before any of it leaves.

pub(crate) mod git;
pub(crate) mod prisma;

use super::{
    inspection::bounded_read,
    process::{self, ProcessError},
    sandbox::{self, Fence, Isolation},
    source, tool_paths,
};
use crate::{
    application::ports::{
        build::{BuildProgress, BuildService},
        catalog::EngineError,
        providers::{Artifact, Asset},
    },
    domain::{
        Secret,
        leaks::LeakScanner,
        model::{Plan, browser_env},
        provenance::SourceProvenance,
    },
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
    time::Duration,
};
use zeroize::Zeroizing;

/// How long one step of a build (an install, the build script, bundling) may take.
const STEP_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Named in the snapshot's digest, so a change to how it's taken changes every digest.
const SNAPSHOT_FORMAT: &str = "hoista-source-v1";

/// The `.env` files a build reads values from, later ones winning.
const ENV_FILES: [&str; 4] = [
    ".env",
    ".env.local",
    ".env.production",
    ".env.production.local",
];
const MAX_ENV_FILE_BYTES: u64 = 256 * 1024;
/// Set for every build and every service; not secrets.
const RUNTIME_VARIABLES: [&str; 3] = ["NODE_ENV", "PORT", "HOST"];

/// Never copied into a build: dependencies, history, build output and
/// credentials. `.env` files are left out too.
const SNAPSHOT_SKIPS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".hoista",
    ".next",
    ".open-next",
    ".svelte-kit",
    ".wrangler",
    WORKER_OUTPUT,
    ".aws",
    ".ssh",
    ".netrc",
];
const MAX_SNAPSHOT_FILES: u32 = 20_000;
const MAX_SNAPSHOT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;

const MAX_ASSETS: usize = 20_000;
const MAX_ASSET_BYTES: usize = 256 * 1024 * 1024;
const MAX_WORKER_BYTES: usize = 10 * 1024 * 1024;

/// Where the Worker bundle is written, inside the build's workspace.
const WORKER_OUTPUT: &str = ".hoista-worker";
/// The Worker's file name when the build doesn't make one.
const DEFAULT_WORKER_NAME: &str = "main.mjs";
/// Frameworks that build a Worker.
const EDGE_FRAMEWORKS: [&str; 5] = ["tanstack-start", "nextjs", "sveltekit", "hono", "worker"];
/// Frameworks whose Worker serves everything, with no separate files.
const WORKER_ONLY_FRAMEWORKS: [&str; 2] = ["hono", "worker"];
const PACKAGE_MANAGERS: [&str; 4] = ["npm", "pnpm", "yarn", "bun"];
const WRANGLER_FILES: [&str; 3] = ["wrangler.toml", "wrangler.json", "wrangler.jsonc"];

/// The port a container listens on when Hoista writes its Dockerfile.
const CONTAINER_PORT: u16 = 8080;
const NODE_IMAGE: &str = "node:24-bookworm-slim";
const MAX_DOCKERIGNORE_BYTES: u64 = 1024 * 1024;
/// Prisma needs a database address to generate its client, but never
/// connects; this one goes nowhere.
const PRISMA_GENERATE_URL: &str = "postgresql://hoista:generate@127.0.0.1:1/hoista";
/// The Prisma config Hoista writes for Prisma 7, beside the project's own.
const PRISMA_CONFIG: &str = ".hoista-prisma.config.ts";

/// Builds projects in workspaces under the state folder's `builds`.
pub struct LocalBuilder {
    root: PathBuf,
}

impl LocalBuilder {
    /// A builder keeping its workspaces under `state`.
    pub fn new(state: &Path) -> Self {
        Self {
            root: state.join("builds"),
        }
    }

    fn workspace(&self, id: &str) -> Result<PathBuf, EngineError> {
        uuid::Uuid::parse_str(id)
            .map_err(|_| EngineError::Invalid("Invalid deployment workspace id."))?;
        Ok(self.root.join(id))
    }
}

struct IgnoreProgress;

#[async_trait::async_trait]
impl BuildProgress for IgnoreProgress {
    async fn source_snapshot(&mut self, _: &SourceProvenance) -> Result<(), EngineError> {
        Ok(())
    }
}

fn failure(message: &'static str) -> EngineError {
    EngineError::Invalid(message)
}

fn scanner() -> Result<LeakScanner, EngineError> {
    LeakScanner::new().map_err(|_| EngineError::Store)
}

fn leaked() -> EngineError {
    EngineError::Provider {
        evidence: None,
        code: "deploy.secret_leak".into(),
        message: "A secret was found in source changes or public output. Remove it, rotate the exposed credential and build again.".into(),
    }
}

/// A variable whose value must never turn up in what's published.
fn is_private(name: &str) -> bool {
    !browser_env(name) && !RUNTIME_VARIABLES.contains(&name)
}

/// Copies of the private values, for the leak checks.
fn private_values(secrets: &[(String, Secret)]) -> Vec<Secret> {
    secrets
        .iter()
        .filter(|(name, _)| is_private(name))
        .map(|(_, value)| Secret::new(value.expose().into()))
        .collect()
}

/// Whether `path` resolves to somewhere inside `workspace`.
fn inside(workspace: &Path, path: &Path) -> Result<bool, EngineError> {
    let root = std::fs::canonicalize(workspace).map_err(|_| EngineError::Store)?;
    Ok(path.starts_with(root))
}

#[async_trait::async_trait]
impl BuildService for LocalBuilder {
    fn isolation(&self) -> Result<&'static str, &'static str> {
        match sandbox::isolation() {
            Isolation::Unavailable(reason) => Err(reason),
            how => Ok(how.name()),
        }
    }

    async fn secrets(&self, plan: &Plan) -> Result<Vec<(String, Secret)>, EngineError> {
        self.secrets_with_overrides(plan, vec![]).await
    }

    async fn secrets_with_overrides(
        &self,
        plan: &Plan,
        overrides: Vec<(String, Secret)>,
    ) -> Result<Vec<(String, Secret)>, EngineError> {
        let resolved = source::resolve(&plan.source).await?;
        let root = resolved.path().to_path_buf();
        tokio::task::spawn_blocking(move || {
            let _source = resolved;
            let scanner = scanner()?;
            let mut values = BTreeMap::new();
            let override_names: BTreeSet<_> =
                overrides.iter().map(|(name, _)| name.clone()).collect();
            for filename in ENV_FILES {
                let path = root.join(filename);
                if !path.exists() {
                    continue;
                }
                let metadata = std::fs::symlink_metadata(&path)
                    .map_err(|_| failure("Cannot inspect an environment file."))?;
                if !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.len() > MAX_ENV_FILE_BYTES
                {
                    return Err(failure(
                        "Environment files must be local regular files under 256 KiB.",
                    ));
                }
                for entry in dotenvy::from_path_iter(path)
                    .map_err(|_| failure("Cannot read a project environment file."))?
                {
                    let (name, value) = entry
                        .map_err(|_| failure("A project environment file has invalid syntax."))?;
                    if override_names.contains(&name) {
                        continue;
                    }
                    let secret = Secret::new(value);
                    if !scanner.public_env(&name, &secret).is_empty() {
                        return Err(leaked());
                    }
                    values.insert(name, secret);
                }
            }
            for (name, value) in overrides {
                if !scanner.public_env(&name, &value).is_empty() {
                    return Err(leaked());
                }
                values.insert(name, value);
            }
            Ok(values.into_iter().collect())
        })
        .await
        .map_err(|_| EngineError::Store)?
    }

    async fn build(
        &self,
        plan: &Plan,
        deploy_id: &str,
        secrets: Vec<(String, Secret)>,
    ) -> Result<Artifact, EngineError> {
        self.build_with_progress(plan, deploy_id, secrets, &mut IgnoreProgress)
            .await
    }

    async fn build_with_progress(
        &self,
        plan: &Plan,
        deploy_id: &str,
        secrets: Vec<(String, Secret)>,
        progress: &mut dyn BuildProgress,
    ) -> Result<Artifact, EngineError> {
        let resolved = source::resolve(&plan.source).await?;
        let root = resolved.path().to_path_buf();
        let workspace = self.workspace(deploy_id)?;
        let known = private_values(&secrets);
        let context = git::check(&root, &known).await?;
        let snapshot = workspace.clone();
        let source_root = root.clone();
        let (snapshot_sha256, snapshot_files) =
            tokio::task::spawn_blocking(move || snapshot_source(&source_root, &snapshot, &known))
                .await
                .map_err(|_| EngineError::Store)??;
        if git::observe(&root).await? != context {
            return Err(failure(
                "Source Git state changed while copying build inputs. Retry from a stable working tree.",
            ));
        }
        let provenance = SourceProvenance {
            git_commit: context.commit,
            git_dirty: context.dirty,
            snapshot_sha256,
            snapshot_files,
            snapshot_format: SNAPSHOT_FORMAT.into(),
        };
        progress.source_snapshot(&provenance).await?;
        // A provider that runs containers builds the image itself.
        if plan
            .resources
            .iter()
            .any(|resource| super::providers::runs_containers(resource.provider))
        {
            return container_context(plan, &workspace, secrets, provenance).await;
        }
        prisma::prepare(plan, &workspace).await?;
        if !plan.build_command.is_empty() {
            check_build_command(&plan.build_command)?;
            // Only the locked dependencies are installed, with install scripts
            // off; provider tokens are never passed to a build.
            let manager = &plan.build_command[0];
            if !workspace.join("node_modules").exists() {
                install_locked(manager, &workspace).await?;
            }
            run(
                manager,
                &["run", "build"],
                &workspace,
                &secrets,
                STEP_TIMEOUT,
                "The folder's build script failed. Run its build command there to see why; build output isn't kept because it may contain secrets.",
            )
            .await?;
        }
        if plan.framework == "nextjs" && !workspace.join(".open-next/worker.js").is_file() {
            let adapter = workspace.join("node_modules/@opennextjs/cloudflare/dist/cli/index.js");
            if !adapter.is_file() || !workspace.join(".next/BUILD_ID").is_file() {
                return Err(failure(
                    "Next.js requires a successful next build and a locked @opennextjs/cloudflare adapter.",
                ));
            }
            run(
                "node",
                &[
                    adapter.to_str().ok_or(EngineError::Store)?,
                    "build",
                    "--skipNextBuild",
                ],
                &workspace,
                &secrets,
                STEP_TIMEOUT,
                "The Next.js Cloudflare adapter failed. Run npx @opennextjs/cloudflare build in the folder to see why.",
            )
            .await?;
        }
        let edge = EDGE_FRAMEWORKS.contains(&plan.framework.as_str());
        let entry_name = if edge {
            bundle_worker(&plan.framework, &workspace).await?
        } else {
            DEFAULT_WORKER_NAME.to_owned()
        };
        let output_directory = Path::new(&plan.output_directory);
        if output_directory.is_absolute()
            || output_directory.components().any(|part| {
                matches!(
                    part,
                    Component::ParentDir | Component::Prefix(_) | Component::RootDir
                )
            })
        {
            return Err(failure(
                "Build output must remain inside the deployment workspace.",
            ));
        }
        let output = std::fs::canonicalize(workspace.join(output_directory))
            .map_err(|_| failure("The build did not produce its configured asset directory."))?;
        if !inside(&workspace, &output)? {
            return Err(failure("Build output escapes the deployment workspace."));
        }
        if edge {
            let bundle = std::fs::canonicalize(workspace.join(WORKER_OUTPUT))
                .map_err(|_| failure("Wrangler did not produce its bundle directory."))?;
            if !inside(&workspace, &bundle)? {
                return Err(failure("Worker output escapes the deployment workspace."));
            }
        }
        let worker_only = WORKER_ONLY_FRAMEWORKS.contains(&plan.framework.as_str());
        tokio::task::spawn_blocking(move || {
            let scanner = scanner()?;
            let known = private_values(&secrets);
            let assets = read_assets(&output, &scanner, &known, AssetScope::Public)?;
            let mut worker_modules = if edge {
                read_assets(
                    &workspace.join(WORKER_OUTPUT),
                    &scanner,
                    &known,
                    AssetScope::Worker,
                )?
            } else {
                vec![]
            };
            let worker = if edge {
                let index = worker_modules
                    .iter()
                    .position(|asset| asset.path == format!("/{entry_name}"))
                    .ok_or_else(|| {
                        failure("Wrangler did not produce the expected Worker entry module.")
                    })?;
                Some(worker_modules.remove(index).bytes)
            } else {
                None
            };
            Ok(Artifact {
                provenance: Some(provenance),
                assets: if worker_only { vec![] } else { assets },
                worker,
                worker_name: entry_name,
                worker_modules,
                image: None,
                secrets: secrets
                    .into_iter()
                    .filter(|(name, _)| is_private(name))
                    .collect(),
            })
        })
        .await
        .map_err(|_| EngineError::Store)?
    }

    async fn migrate(
        &self,
        plan: &Plan,
        deploy_id: &str,
        database: &Secret,
    ) -> Result<(), EngineError> {
        prisma::migrate(plan, &self.workspace(deploy_id)?, database).await
    }

    async fn cleanup(&self, deploy_id: &str) -> Result<(), EngineError> {
        let path = self.workspace(deploy_id)?;
        match tokio::fs::remove_dir_all(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(EngineError::Store),
        }
    }
}

/// Readies the workspace as a container build context for a provider that
/// builds the image itself: a Dockerfile (written for a Node project that
/// has none), checked for secrets, with local-only files kept out.
async fn container_context(
    plan: &Plan,
    workspace: &Path,
    secrets: Vec<(String, Secret)>,
    provenance: SourceProvenance,
) -> Result<Artifact, EngineError> {
    if !workspace.join("Dockerfile").is_file() {
        write_node_dockerfile(plan, workspace).await?;
    }
    let scan_root = workspace.to_path_buf();
    let known = private_values(&secrets);
    tokio::task::spawn_blocking(move || {
        read_assets(&scan_root, &scanner()?, &known, AssetScope::BuildContext).map(|_| ())
    })
    .await
    .map_err(|_| EngineError::Store)??;
    prisma::prepare(plan, workspace).await?;
    // Migration tooling runs here, never in the image.
    let ignore = workspace.join(".dockerignore");
    let mut ignored = if ignore.exists() {
        String::from_utf8(bounded_read(&ignore, MAX_DOCKERIGNORE_BYTES)?)
            .map_err(|_| failure("Docker ignore configuration must be UTF-8."))?
    } else {
        String::new()
    };
    ignored.push_str(&format!("\nnode_modules\n.env*\n!{PRISMA_CONFIG}\n"));
    tokio::fs::write(ignore, ignored)
        .await
        .map_err(|_| EngineError::Store)?;
    Ok(Artifact {
        provenance: Some(provenance),
        assets: vec![],
        worker: None,
        worker_name: DEFAULT_WORKER_NAME.into(),
        worker_modules: vec![],
        image: Some(workspace.to_string_lossy().into()),
        secrets,
    })
}

/// Writes a Dockerfile for a Node project with an npm lockfile.
async fn write_node_dockerfile(plan: &Plan, workspace: &Path) -> Result<(), EngineError> {
    if !workspace.join("package-lock.json").is_file() {
        return Err(failure(
            "Add a Dockerfile exposing port 8080 for this service.",
        ));
    }
    let package: serde_json::Value = serde_json::from_slice(
        &std::fs::read(workspace.join("package.json")).map_err(|_| EngineError::Store)?,
    )
    .map_err(|_| EngineError::Store)?;
    let schema = plan
        .prisma_migration
        .as_ref()
        .map(|migration| migration.schema.as_str());
    tokio::fs::write(
        workspace.join("Dockerfile"),
        node_dockerfile(&package, schema)?,
    )
    .await
    .map_err(|_| EngineError::Store)
}

/// The Dockerfile for a Node project with this `package.json`. With a Prisma
/// schema, install scripts stay off and the client is generated instead.
fn node_dockerfile(
    package: &serde_json::Value,
    prisma_schema: Option<&str>,
) -> Result<String, EngineError> {
    if !package["scripts"]["start"].is_string() {
        return Err(failure(
            "Add a package start script or a Dockerfile before deploying this service.",
        ));
    }
    let build = if package["scripts"]["build"].is_string() {
        "RUN npm run build\n"
    } else {
        ""
    };
    let (install, generate) = match prisma_schema {
        Some(schema) => {
            let prisma = "node node_modules/prisma/build/index.js generate";
            (
                "RUN npm ci --ignore-scripts\n",
                format!(
                    "RUN export DATABASE_URL={PRISMA_GENERATE_URL} DIRECT_URL={PRISMA_GENERATE_URL}; \
                     if [ -f {PRISMA_CONFIG} ]; \
                     then {prisma} --schema {schema} --config {PRISMA_CONFIG}; \
                     else {prisma} --schema {schema}; fi\n"
                ),
            )
        }
        None => ("RUN npm ci\n", String::new()),
    };
    Ok(format!(
        "FROM {NODE_IMAGE}\n\
         WORKDIR /app\n\
         COPY . .\n\
         {install}{generate}{build}\
         ENV NODE_ENV=production PORT={CONTAINER_PORT} HOST=0.0.0.0\n\
         EXPOSE {CONTAINER_PORT}\n\
         CMD [\"npm\",\"start\"]\n"
    ))
}

/// Only `<manager> run build`, with the package manager that was detected.
fn check_build_command(command: &[String]) -> Result<(), EngineError> {
    match command {
        [manager, run, build]
            if PACKAGE_MANAGERS.contains(&manager.as_str()) && run == "run" && build == "build" =>
        {
            Ok(())
        }
        _ => Err(failure(
            "Build commands must use the detected package manager's build script.",
        )),
    }
}

/// Bundles the framework's Worker entry with Wrangler into [`WORKER_OUTPUT`],
/// and returns the entry's file name.
async fn bundle_worker(framework: &str, workspace: &Path) -> Result<String, EngineError> {
    let candidates: &[&str] = match framework {
        "nextjs" => &[".open-next/worker.js"],
        "sveltekit" => &[".svelte-kit/cloudflare/_worker.js"],
        "tanstack-start" => &[
            "dist/server/index.js",
            "dist/server/server.js",
            ".output/server/index.mjs",
        ],
        _ => &["dist/index.js"],
    };
    let entry = candidates
        .iter()
        .map(|path| workspace.join(path))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            failure("Configure the Cloudflare framework adapter to produce a Worker entry module.")
        })?;
    checked_build_file(workspace, &entry)?;
    reserve_worker_output(workspace)?;
    let wrangler = workspace.join("node_modules/wrangler/bin/wrangler.js");
    // With no build step, nothing has installed the lockfile yet.
    if !wrangler.is_file() && !workspace.join("node_modules").exists() {
        let manager = locked_manager(workspace).ok_or(failure(
            "This folder has no lockfile. Add wrangler to its development dependencies and commit the lockfile in this folder (in a monorepo, the app's own folder).",
        ))?;
        install_locked(manager, workspace).await?;
    }
    if !wrangler.is_file() {
        return Err(failure(
            "Add wrangler to the project's locked development dependencies to bundle its Worker modules.",
        ));
    }
    let entry_name = entry
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(EngineError::Store)?
        .to_owned();
    // The config is passed explicitly: given only an entry path, Wrangler
    // looks for config beside the entry, and that clashes with Vite's own.
    let config = WRANGLER_FILES
        .iter()
        .map(|name| workspace.join(name))
        .find(|path| path.is_file());
    let mut arguments = vec![
        wrangler.to_str().ok_or(EngineError::Store)?,
        "deploy",
        entry.to_str().ok_or(EngineError::Store)?,
        "--dry-run",
        "--outdir",
        WORKER_OUTPUT,
    ];
    if let Some(config) = &config {
        checked_build_file(workspace, config)?;
        arguments.extend(["--config", config.to_str().ok_or(EngineError::Store)?]);
    }
    run(
        "node",
        &arguments,
        workspace,
        &[],
        STEP_TIMEOUT,
        "Wrangler couldn't bundle the Worker. Run wrangler deploy --dry-run in the folder to see why.",
    )
    .await?;
    Ok(entry_name)
}

/// Runs project build code in `directory`, a workspace under `state/builds`,
/// isolated where this computer allows it. The engine refuses builds that
/// can't be isolated unless the person chose to allow them.
async fn run(
    program: &str,
    args: &[&str],
    directory: &Path,
    env: &[(String, Secret)],
    limit: Duration,
    failed: &'static str,
) -> Result<(), EngineError> {
    let resolved = tool_paths::command(program);
    let program_path = resolved.as_std().get_program().to_owned();
    let state = directory
        .parent()
        .filter(|builds| builds.file_name().is_some_and(|name| name == "builds"))
        .and_then(Path::parent)
        .ok_or(EngineError::Store)?;
    let cache = state.join("build-cache");
    std::fs::create_dir_all(&cache).map_err(|_| EngineError::Store)?;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    // The tool folders saved with `hoist tools configure`, so what a tool
    // starts by name (npm starts node, a script starts sh or mkdir) is found
    // as in the person's terminal, and the sandbox lets it be read.
    let path = tool_paths::search_path().or_else(|| std::env::var_os("PATH"));
    let fence = Fence {
        workspace: directory,
        cache: &cache,
        state,
        home: home.as_deref(),
        tools: Fence::tool_folders(path.as_ref(), home.as_deref()),
    };
    // Without a sandbox the engine has already checked the person allows it.
    let mut command = sandbox::fenced(sandbox::isolation(), &program_path, args, &fence)
        .unwrap_or_else(|| {
            let mut plain = resolved;
            plain.args(args);
            plain
        });
    command
        .current_dir(directory)
        .kill_on_drop(true)
        .env_clear()
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    process::inherit_environment(&mut command);
    if let Some(path) = &path {
        command.env("PATH", path);
    }
    command
        .env("CI", "true")
        .env("WRANGLER_SEND_METRICS", "false")
        .env("NEXT_TELEMETRY_DISABLED", "1")
        // Package caches live in Hoista's build cache, inside the fence.
        .env("npm_config_cache", cache.join("npm"))
        .env("XDG_CACHE_HOME", cache.join("xdg"))
        .env("YARN_CACHE_FOLDER", cache.join("yarn"))
        .env("BUN_INSTALL_CACHE_DIR", cache.join("bun"));
    for (name, value) in env {
        command.env(name, value.expose());
    }
    let status = process::run(&mut command, None, limit)
        .await
        .map_err(|error| match error {
            ProcessError::Spawn(_) => {
                failure("The build tool isn't installed, or Hoista can't find it. Install it, run hoist tools configure, then restart Hoista.")
            }
            ProcessError::Timeout => failure("The build took too long and was stopped."),
            ProcessError::Cleanup => {
                failure("The build was stopped, but Hoista couldn't confirm it exited. Check for build tools still running before retrying.")
            }
            ProcessError::Io(_) => failure("The build process failed."),
        })?;
    if !status.success() {
        return Err(failure(failed));
    }
    Ok(())
}

/// The package manager a folder's lockfile belongs to.
fn locked_manager(workspace: &Path) -> Option<&'static str> {
    [
        ("package-lock.json", "npm"),
        ("npm-shrinkwrap.json", "npm"),
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("bun.lock", "bun"),
        ("bun.lockb", "bun"),
    ]
    .into_iter()
    .find(|(file, _)| workspace.join(file).is_file())
    .map(|(_, manager)| manager)
}

/// Installs a folder's locked dependencies, with install scripts off.
async fn install_locked(manager: &str, workspace: &Path) -> Result<(), EngineError> {
    // Yarn has no flag for it, only this setting.
    let no_scripts = [(
        "YARN_ENABLE_SCRIPTS".to_owned(),
        Secret::new("false".into()),
    )];
    let args: &[&str] = match manager {
        "npm" => &["ci", "--ignore-scripts", "--no-audit", "--no-fund"],
        "pnpm" => &["install", "--frozen-lockfile", "--ignore-scripts"],
        "yarn" => &["install", "--immutable"],
        "bun" => &["install", "--frozen-lockfile", "--ignore-scripts"],
        _ => return Err(failure("Unsupported package manager.")),
    };
    run(
        manager,
        args,
        workspace,
        &no_scripts,
        STEP_TIMEOUT,
        "Installing the folder's locked dependencies failed. Run the same clean install there (npm ci, pnpm install --frozen-lockfile, yarn install --immutable or bun install --frozen-lockfile) and fix what it reports; build output isn't kept because it may contain secrets.",
    )
    .await
}

/// Copies the source into `destination`, checking each file for secrets,
/// and returns the copy's digest and file count. The digest covers every
/// file's path, contents and whether it's executable, in a fixed order.
fn snapshot_source(
    root: &Path,
    destination: &Path,
    secrets: &[Secret],
) -> Result<(String, u32), EngineError> {
    let mut digest = Sha256::new();
    digest.update(SNAPSHOT_FORMAT.as_bytes());
    digest.update([0]);
    let scanner = scanner()?;
    if destination.exists() {
        return Err(failure(
            "This deployment workspace already exists; queue a new deployment.",
        ));
    }
    if destination.starts_with(root) {
        return Err(failure(
            "The source directory cannot contain Hoista's build directory.",
        ));
    }
    std::fs::create_dir_all(destination)
        .map_err(|_| failure("Cannot create a private build workspace."))?;
    let mut bytes = 0_u64;
    let mut files = 0;
    for entry in walkdir::WalkDir::new(root)
        .sort_by(|a, b| {
            a.file_name()
                .to_string_lossy()
                .cmp(&b.file_name().to_string_lossy())
        })
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !SNAPSHOT_SKIPS.contains(&name.as_ref()) && !name.starts_with(".env")
        })
    {
        let entry = entry.map_err(|_| failure("Cannot snapshot the project source."))?;
        if entry.file_type().is_symlink() {
            return Err(failure("Source snapshots cannot include symbolic links."));
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| EngineError::Store)?;
        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target).map_err(|_| EngineError::Store)?;
        } else if entry.file_type().is_file() {
            files += 1;
            let metadata = entry.metadata().map_err(|_| EngineError::Store)?;
            if files > MAX_SNAPSHOT_FILES {
                return Err(failure(
                    "The source snapshot exceeds 20,000 files or 512 MiB.",
                ));
            }
            let content = Zeroizing::new(bounded_read(entry.path(), MAX_FILE_BYTES)?);
            if !scanner.scan("source", &content, secrets).is_empty() {
                return Err(leaked());
            }
            bytes += content.len() as u64;
            if bytes > MAX_SNAPSHOT_BYTES {
                return Err(failure("The source snapshot exceeds 512 MiB."));
            }
            let name = relative
                .to_str()
                .ok_or_else(|| failure("Source paths must be valid UTF-8."))?
                .replace(std::path::MAIN_SEPARATOR, "/");
            digest.update((name.len() as u64).to_be_bytes());
            digest.update(name.as_bytes());
            digest.update((content.len() as u64).to_be_bytes());
            digest.update(&content[..]);
            #[cfg(unix)]
            let executable = {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o111 != 0
            };
            #[cfg(not(unix))]
            let executable = false;
            digest.update([u8::from(executable)]);
            std::fs::write(&target, &content[..])
                .map_err(|_| failure("Could not copy a project source file."))?;
            std::fs::set_permissions(&target, metadata.permissions())
                .map_err(|_| failure("Cannot preserve source file permissions."))?;
        }
    }
    Ok((format!("{:x}", digest.finalize()), files))
}

/// What a set of files is for, which decides what's skipped and the limits.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AssetScope {
    /// Files served to anyone.
    Public,
    /// The Worker bundle.
    Worker,
    /// A container build context, read only to check it for secrets.
    BuildContext,
}

/// Reads every file under `root`, refusing links, files over the limits and
/// anything carrying a secret.
fn read_assets(
    root: &Path,
    scanner: &LeakScanner,
    secrets: &[Secret],
    scope: AssetScope,
) -> Result<Vec<Asset>, EngineError> {
    let mut assets = Vec::new();
    let mut total = 0;
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|_| failure("Cannot read build assets."))?;
        if entry.file_type().is_symlink() {
            return Err(failure("Public assets cannot be symbolic links."));
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| EngineError::Store)?;
        if scope != AssetScope::BuildContext
            && relative
                .components()
                .any(|part| part.as_os_str().to_string_lossy().starts_with('.'))
        {
            continue;
        }
        // Source maps aren't uploaded, so they don't count towards the limit.
        if scope == AssetScope::Worker
            && relative
                .extension()
                .is_some_and(|extension| extension == "map")
        {
            continue;
        }
        if scope == AssetScope::Public
            && relative.components().any(|part| {
                matches!(
                    part.as_os_str().to_str(),
                    Some("_worker.js" | "_routes.json")
                )
            })
        {
            continue;
        }
        if entry.metadata().map_err(|_| EngineError::Store)?.len() > MAX_FILE_BYTES {
            return Err(failure("An asset exceeds 25 MiB."));
        }
        let bytes = std::fs::read(entry.path()).map_err(|_| failure("Cannot read an asset."))?;
        total += bytes.len();
        if bytes.len() as u64 > MAX_FILE_BYTES
            || total > MAX_ASSET_BYTES
            || assets.len() >= MAX_ASSETS
        {
            return Err(failure("Public assets exceed the provider upload limits."));
        }
        if scope == AssetScope::Worker && total > MAX_WORKER_BYTES {
            return Err(failure(
                "The Worker bundle exceeds the conservative 10 MiB upload limit.",
            ));
        }
        let path = format!("/{}", relative.to_string_lossy().replace('\\', "/"));
        if !scanner.scan(&path, &bytes, secrets).is_empty() {
            return Err(leaked());
        }
        assets.push(Asset {
            path,
            content_type: mime_guess::from_path(entry.path())
                .first_or_octet_stream()
                .to_string(),
            bytes,
        });
    }
    Ok(assets)
}

/// Refuses a build that made something at [`WORKER_OUTPUT`] itself.
fn reserve_worker_output(workspace: &Path) -> Result<(), EngineError> {
    match std::fs::symlink_metadata(workspace.join(WORKER_OUTPUT)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(failure("Cannot inspect Worker output.")),
        Ok(_) => Err(failure(
            "The build created Hoista's reserved Worker output path. Remove that output from the build script.",
        )),
    }
}

/// Refuses a Worker entry or config that's a link or outside the workspace.
fn checked_build_file(workspace: &Path, path: &Path) -> Result<(), EngineError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| failure("Cannot inspect the Worker entry or configuration."))?;
    let root = std::fs::canonicalize(workspace).map_err(|_| EngineError::Store)?;
    let resolved = std::fs::canonicalize(path)
        .map_err(|_| failure("Cannot resolve the Worker entry or configuration."))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || !resolved.starts_with(root) {
        return Err(failure(
            "Worker entries and configuration must be regular files inside the deployment workspace.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod output_tests {
    use super::{checked_build_file, node_dockerfile, reserve_worker_output};

    #[test]
    fn a_node_project_gets_a_dockerfile_that_runs_its_start_script() {
        let package = serde_json::json!({"scripts": {"start": "node server.js", "build": "tsc"}});
        assert_eq!(
            node_dockerfile(&package, None).unwrap(),
            "FROM node:24-bookworm-slim\nWORKDIR /app\nCOPY . .\nRUN npm ci\nRUN npm run build\n\
             ENV NODE_ENV=production PORT=8080 HOST=0.0.0.0\nEXPOSE 8080\nCMD [\"npm\",\"start\"]\n"
        );
        let with_prisma = node_dockerfile(&package, Some("prisma/schema.prisma")).unwrap();
        assert!(with_prisma.contains("RUN npm ci --ignore-scripts\nRUN export DATABASE_URL=postgresql://hoista:generate@127.0.0.1:1/hoista DIRECT_URL=postgresql://hoista:generate@127.0.0.1:1/hoista; if [ -f .hoista-prisma.config.ts ]; then node node_modules/prisma/build/index.js generate --schema prisma/schema.prisma --config .hoista-prisma.config.ts; else node node_modules/prisma/build/index.js generate --schema prisma/schema.prisma; fi\nRUN npm run build\n"));
        assert!(node_dockerfile(&serde_json::json!({}), None).is_err());
    }

    #[test]
    fn reserved_bundle_paths_reject_existing_and_dangling_outputs() {
        let workspace = tempfile::tempdir().unwrap();
        assert!(reserve_worker_output(workspace.path()).is_ok());
        let output = workspace.path().join(super::WORKER_OUTPUT);
        std::fs::create_dir(&output).unwrap();
        assert!(reserve_worker_output(workspace.path()).is_err());
        std::fs::remove_dir(&output).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(workspace.path().join("missing"), &output).unwrap();
            assert!(reserve_worker_output(workspace.path()).is_err());
        }
    }

    #[test]
    fn worker_inputs_are_local_regular_files() {
        let workspace = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let entry = workspace.path().join("entry.js");
        std::fs::write(&entry, "export default {}").unwrap();
        std::fs::write(
            external.path().join("outside.js"),
            "private fixture content",
        )
        .unwrap();
        assert!(checked_build_file(workspace.path(), &entry).is_ok());
        assert!(checked_build_file(workspace.path(), workspace.path()).is_err());
        assert!(checked_build_file(workspace.path(), &external.path().join("outside.js")).is_err());
        #[cfg(unix)]
        {
            let link = workspace.path().join("linked.js");
            std::os::unix::fs::symlink(&entry, &link).unwrap();
            assert!(checked_build_file(workspace.path(), &link).is_err());
            let directory = workspace.path().join("generated");
            std::os::unix::fs::symlink(external.path(), &directory).unwrap();
            assert!(checked_build_file(workspace.path(), &directory.join("outside.js")).is_err());
        }
    }
}
