//! Reading a project's source to plan its deploy: which framework it uses,
//! how it builds, and which variables it reads. Nothing in it is run, and
//! `.env` files are skipped so their values are never read.

use crate::{
    application::{
        deploys::planning::{Detection, NO_SUPPORTED_PROJECT},
        engine::{SourceGit, temporary_folder},
        ports::{build::SourceInspector, catalog::EngineError},
    },
    domain::{
        model::{PrismaMigration, Source, valid_env_name},
        provenance::SourceObservation,
        schedules,
    },
};
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};

/// The most of any one file that's read.
const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// The most read across the whole project.
const MAX_TOTAL_BYTES: u64 = 32 * 1024 * 1024;
const MAX_FILES: usize = 10_000;

/// Folders of dependencies, history and build output, never the project's own code.
const SKIPPED_FOLDERS: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    ".next",
    ".open-next",
    ".svelte-kit",
    ".wrangler",
    ".hoista-worker",
    "dist",
    "build",
];
const SCRIPT_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs"];
/// Files read for variable names, besides scripts.
const OTHER_EXTENSIONS: &[&str] = &["py", "toml", "json"];
/// The one `.env` file that's read: it lists names, not real values.
const ENV_EXAMPLE: &str = ".env.example";
const WRANGLER_FILES: [&str; 3] = ["wrangler.toml", "wrangler.json", "wrangler.jsonc"];

/// Inspects a selected local source directory.
pub struct LocalInspector;

#[async_trait::async_trait]
impl SourceInspector for LocalInspector {
    async fn observe_source(&self, source: &Source) -> Result<SourceObservation, EngineError> {
        // Not worth a GitHub checkout just to describe the source.
        if matches!(source, Source::Github { .. }) {
            return Ok(SourceObservation::RemoteSource);
        }
        let resolved = super::source::resolve(source).await?;
        let context = super::build::git::observe(resolved.path()).await?;
        Ok(match context.dirty {
            Some(dirty) => SourceObservation::LocalGit {
                commit: context.commit,
                dirty,
            },
            None => SourceObservation::LocalFolder,
        })
    }

    async fn github_remote(&self, source: &Source) -> Option<SourceGit> {
        let Source::Folder { path } = source else {
            return None;
        };
        let remote = super::build::git::github_remote(std::path::Path::new(path)).await?;
        Some(SourceGit {
            repository: remote.repository,
            branch: remote.branch,
            commit: remote.commit,
            temporary: temporary_folder(path),
        })
    }

    async fn detect(&self, source: &Source) -> Result<Detection, EngineError> {
        let source = super::source::resolve(source).await?;
        tokio::task::spawn_blocking(move || detect(source.path()))
            .await
            .map_err(|_| EngineError::Store)?
    }
}

/// Works out how to deploy the project in `root`.
///
/// # Errors
/// Fails for a missing folder, a project over the size limits, invalid
/// configuration, or a project Hoista can't deploy.
pub fn detect(root: &Path) -> Result<Detection, EngineError> {
    if !root.is_dir() {
        return Err(EngineError::Invalid("Choose an existing app directory."));
    }
    let schedules = detect_schedules(root)?;
    let manifest = root.join("package.json");
    let package: Value = if manifest.exists() {
        serde_json::from_slice(&bounded_read(&manifest, MAX_FILE_BYTES)?)
            .map_err(|_| EngineError::Invalid("package.json is not valid JSON."))?
    } else {
        Value::Null
    };
    let mut dependencies = BTreeSet::new();
    for section in ["dependencies", "devDependencies"] {
        if let Some(values) = package.get(section).and_then(Value::as_object) {
            dependencies.extend(values.keys().cloned());
        }
    }
    let has = |name: &str| dependencies.contains(name);
    let (framework, mut shape, output) = if has("@tanstack/react-start") || has("@tanstack/start") {
        ("tanstack-start", "edge", "dist/client")
    } else if has("next") {
        ("nextjs", "edge", ".open-next/assets")
    } else if has("astro") {
        ("astro", "static", "dist")
    } else if has("@sveltejs/kit") {
        ("sveltekit", "edge", ".svelte-kit/cloudflare")
    } else if has("vite") {
        ("vite", "static", "dist")
    } else if has("hono") {
        ("hono", "edge", "dist")
    } else if has("express") || has("fastify") || has("koa") {
        ("node", "service", ".")
    } else if root.join("requirements.txt").is_file() || root.join("pyproject.toml").is_file() {
        ("python", "service", ".")
    } else if root.join("Cargo.toml").is_file() {
        ("rust", "service", ".")
    } else if root.join("go.mod").is_file() {
        ("go", "service", ".")
    } else if WRANGLER_FILES.iter().any(|name| root.join(name).is_file()) {
        (
            "worker",
            if schedules.is_some() { "job" } else { "edge" },
            "dist",
        )
    } else if root.join("index.html").is_file() {
        ("static", "static", ".")
    } else {
        return Err(EngineError::Invalid(NO_SUPPORTED_PROJECT));
    };
    let mut warnings = Vec::new();
    if schedules.as_ref().is_some_and(|values| !values.is_empty()) && shape == "static" {
        return Err(EngineError::Invalid(
            "Cron triggers require a Worker scheduled handler, not a static asset deployment.",
        ));
    }
    if has("@supabase/supabase-js") {
        shape = "supabase";
        warnings.push(
            "Keep the existing Supabase project, auth and storage; only the frontend is deployed."
                .into(),
        );
    }
    if dependencies.iter().any(|d| d.contains("base44")) {
        return Err(EngineError::Invalid(
            "Base44-managed backends cannot be deployed through a public provider API.",
        ));
    }
    if dependencies.iter().any(|d| d.contains("lovable")) {
        warnings.push("Lovable-managed backend storage, functions and secrets are not included in source exports.".into());
    }
    if framework == "nextjs" && !has("@opennextjs/cloudflare") {
        return Err(EngineError::Invalid(
            "Install and configure @opennextjs/cloudflare before planning a Next.js Worker.",
        ));
    }
    let manager = if !package.is_null() {
        Some(
            if root.join("pnpm-lock.yaml").exists() {
                "pnpm"
            } else if root.join("yarn.lock").exists() {
                "yarn"
            } else if root.join("bun.lock").exists() || root.join("bun.lockb").exists() {
                "bun"
            } else {
                "npm"
            }
            .to_owned(),
        )
    } else {
        None
    };
    let build_command = if package
        .pointer("/scripts/build")
        .and_then(Value::as_str)
        .is_some()
    {
        vec![
            manager.clone().unwrap_or_else(|| "npm".into()),
            "run".into(),
            "build".into(),
        ]
    } else {
        Vec::new()
    };
    let mut names = BTreeSet::new();
    let env_pattern =
        regex::Regex::new(r"(?:process\.env\.|import\.meta\.env\.)([A-Za-z_][A-Za-z0-9_]*)")
            .map_err(|_| EngineError::Store)?;
    let python_env_pattern =
        regex::Regex::new(r#"(?:os\.getenv\s*\(|os\.environ\s*\.\s*get\s*\(|os\.environ\s*\[)\s*["']([A-Za-z_][A-Za-z0-9_]*)["']\s*[,\)\]]"#)
            .map_err(|_| EngineError::Store)?;
    let indexed_env_pattern = regex::Regex::new(
        r#"(?:process\.env|import\.meta\.env)\s*\[\s*["']([A-Za-z_][A-Za-z0-9_]*)["']\s*\]"#,
    )
    .map_err(|_| EngineError::Store)?;
    let mut total = 0_u64;
    let mut count = 0;
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            !SKIPPED_FOLDERS.contains(&entry.file_name().to_string_lossy().as_ref())
        })
    {
        let entry =
            entry.map_err(|_| EngineError::Invalid("Could not inspect the source directory."))?;
        if !entry.file_type().is_file() {
            continue;
        }
        count += 1;
        if count > MAX_FILES {
            return Err(EngineError::Invalid(
                "Source inspection exceeds 10,000 files. Choose the project subdirectory.",
            ));
        }
        let file_name = entry.file_name().to_string_lossy();
        let env_example = file_name == ENV_EXAMPLE;
        if file_name.starts_with(".env") && !env_example {
            continue;
        }
        let extension = entry
            .path()
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or("");
        let script = SCRIPT_EXTENSIONS.contains(&extension);
        if !env_example && !script && !OTHER_EXTENSIONS.contains(&extension) {
            continue;
        }
        let bytes = bounded_read(entry.path(), MAX_FILE_BYTES)?;
        total += bytes.len() as u64;
        if total > MAX_TOTAL_BYTES {
            return Err(EngineError::Invalid(
                "Source inspection exceeds 32 MiB. Choose a narrower source directory.",
            ));
        }
        let text = String::from_utf8_lossy(&bytes);
        for captures in env_pattern.captures_iter(&text) {
            if let Some(name) = captures.get(1) {
                names.insert(name.as_str().into());
            }
        }
        let pattern = if extension == "py" {
            Some(&python_env_pattern)
        } else if script {
            Some(&indexed_env_pattern)
        } else {
            None
        };
        if let Some(pattern) = pattern {
            for captures in pattern.captures_iter(&text) {
                if let Some(name) = captures.get(1) {
                    names.insert(name.as_str().into());
                }
            }
        }
        if env_example {
            for line in text.lines() {
                if let Some((name, _)) = line
                    .trim()
                    .strip_prefix("export ")
                    .unwrap_or(line.trim())
                    .split_once('=')
                {
                    let name = name.trim();
                    if valid_env_name(name) {
                        names.insert(name.into());
                    }
                }
            }
        }
    }
    names.remove("NODE_ENV");
    let postgres = !has("@supabase/supabase-js")
        && (has("@prisma/client")
            || has("pg")
            || has("postgres")
            || has("@neondatabase/serverless"));
    let prisma_migration = if has("@prisma/client") && !has("@supabase/supabase-js") {
        if !has("prisma") {
            return Err(EngineError::Invalid(
                "Add the Prisma CLI to the project's locked dependencies before deploying migrations.",
            ));
        }
        let schema = package
            .pointer("/prisma/schema")
            .and_then(Value::as_str)
            .unwrap_or("prisma/schema.prisma");
        let migrations = schema
            .rsplit_once('/')
            .map(|(parent, _)| format!("{parent}/migrations"))
            .unwrap_or_else(|| "migrations".into());
        let migration = PrismaMigration {
            schema: schema.into(),
            migrations,
        };
        super::build::prisma::validate(root, &migration)?;
        warnings.push("Checked-in Prisma migrations run against the selected database before code release. Code rollback never reverses database changes.".into());
        Some(migration)
    } else {
        None
    };
    let generated_secret_names = if has("better-auth") {
        names.insert("BETTER_AUTH_SECRET".into());
        vec!["BETTER_AUTH_SECRET".into()]
    } else {
        vec![]
    };
    Ok(Detection {
        schedules,
        generated_secret_names,
        prisma_migration,
        framework: framework.into(),
        shape: shape.into(),
        package_manager: manager,
        build_command,
        output_directory: output.into(),
        env_names: names.into_iter().collect(),
        postgres,
        warnings,
    })
}

/// Reads a file of the project, refusing one over `limit` bytes.
pub(crate) fn bounded_read(path: &Path, limit: u64) -> Result<Vec<u8>, EngineError> {
    hoista_system::files::read_bounded(path, limit).map_err(|error| match error {
        hoista_system::files::FileError::TooLarge => {
            EngineError::Invalid("A source configuration file exceeds the inspection limit.")
        }
        _ => EngineError::Invalid("Could not read a project source file."),
    })
}

/// The cron schedules a Wrangler file sets in `triggers.crons`. `None` when
/// it doesn't say, which leaves the schedules already deployed as they are.
fn detect_schedules(root: &Path) -> Result<Option<Vec<String>>, EngineError> {
    let [toml, json, jsonc] = WRANGLER_FILES.map(|name| root.join(name));
    if [&toml, &json, &jsonc]
        .iter()
        .filter(|path| path.exists())
        .count()
        > 1
    {
        return Err(EngineError::Invalid(
            "Keep one Wrangler configuration file before planning.",
        ));
    }
    let value: Value = if toml.exists() {
        let bytes = bounded_read(&toml, MAX_FILE_BYTES)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| EngineError::Invalid("Wrangler configuration must be UTF-8."))?;
        let parsed: toml::Value = toml::from_str(text)
            .map_err(|_| EngineError::Invalid("wrangler.toml is not valid TOML."))?;
        serde_json::to_value(parsed).map_err(|_| EngineError::Store)?
    } else if json.exists() {
        serde_json::from_slice(&bounded_read(&json, MAX_FILE_BYTES)?)
            .map_err(|_| EngineError::Invalid("wrangler.json must be strict JSON."))?
    } else if jsonc.exists() {
        super::jsonc::parse(&bounded_read(&jsonc, MAX_FILE_BYTES)?)?
    } else {
        return Ok(None);
    };
    let Some(triggers) = value.get("triggers") else {
        return Ok(None);
    };
    let triggers = triggers
        .as_object()
        .ok_or(EngineError::Invalid("Wrangler triggers must be an object."))?;
    let Some(crons) = triggers.get("crons") else {
        return Ok(None);
    };
    let expressions: Vec<String> = serde_json::from_value(crons.clone())
        .map_err(|_| EngineError::Invalid("Wrangler crons must be a list of expressions."))?;
    if !schedules::valid(&expressions) {
        return Err(EngineError::Invalid(
            "Use up to 100 distinct supported five-field UTC cron expressions.",
        ));
    }
    Ok(Some(expressions))
}
