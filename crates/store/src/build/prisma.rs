//! A project's checked-in Prisma migrations for PostgreSQL, run with the
//! Prisma CLI from its own locked dependencies. Hoista never generates,
//! resets or reverses a migration.

use super::{PRISMA_CONFIG, PRISMA_GENERATE_URL, STEP_TIMEOUT, failure, install_locked, run};
use crate::{
    application::ports::catalog::EngineError,
    domain::{
        Secret,
        model::{Plan, PrismaMigration},
    },
    inspection::bounded_read,
};
use std::path::{Path, PathBuf};

const CLI_ENTRY: &str = "node_modules/prisma/build/index.js";
const CLI_PACKAGE: &str = "node_modules/prisma/package.json";
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_HISTORY_ENTRIES: usize = 10_000;
const MAX_MIGRATIONS: usize = 1_000;
/// Where Prisma 6 looks for a config of the project's own.
const PRISMA_6_CONFIGS: [&str; 12] = [
    "prisma.config.ts",
    "prisma.config.js",
    "prisma.config.mjs",
    "prisma.config.mts",
    "prisma.config.cjs",
    "prisma.config.cts",
    ".config/prisma.ts",
    ".config/prisma.js",
    ".config/prisma.mjs",
    ".config/prisma.mts",
    ".config/prisma.cjs",
    ".config/prisma.cts",
];

/// Checks the schema and its migrations can be run safely: one PostgreSQL
/// datasource whose addresses come from `DATABASE_URL` or `DIRECT_URL`, no
/// links on the way, and at least one migration checked in.
pub(crate) fn validate(root: &Path, migration: &PrismaMigration) -> Result<(), EngineError> {
    if !migration.valid() {
        return Err(failure(
            "Use a relative Prisma schema path with migrations beside it.",
        ));
    }
    for relative in [&migration.schema, &migration.migrations] {
        let mut path = root.to_path_buf();
        for component in Path::new(relative).components() {
            path.push(component);
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|_| failure("Cannot read Prisma configuration."))?;
            if metadata.file_type().is_symlink() {
                return Err(failure(
                    "Prisma configuration cannot traverse symbolic links.",
                ));
            }
        }
    }
    let schema = bounded_read(&root.join(&migration.schema), MAX_FILE_BYTES)?;
    let schema =
        std::str::from_utf8(&schema).map_err(|_| failure("The Prisma schema must be UTF-8."))?;
    let comments = regex::Regex::new(r"(?m)//[^\n]*").map_err(|_| EngineError::Store)?;
    let schema = comments.replace_all(schema, "");
    let datasource = regex::Regex::new(r"(?s)\bdatasource\s+\w+\s*\{([^}]+)\}")
        .map_err(|_| EngineError::Store)?;
    let sources: Vec<_> = datasource.captures_iter(&schema).collect();
    if sources.len() != 1 {
        return Err(failure(
            "Use exactly one Prisma datasource for automatic migrations.",
        ));
    }
    let schema = &sources[0][1];
    let provider =
        regex::Regex::new(r#"provider\s*=\s*"postgresql""#).map_err(|_| EngineError::Store)?;
    if !provider.is_match(schema) {
        return Err(failure(
            "Automatic Prisma migrations currently require a PostgreSQL datasource.",
        ));
    }
    let env = regex::Regex::new(r#"env\s*\(\s*"([^"]+)"\s*\)"#).map_err(|_| EngineError::Store)?;
    if env
        .captures_iter(schema)
        .any(|capture| !matches!(&capture[1], "DATABASE_URL" | "DIRECT_URL"))
    {
        return Err(failure(
            "Use DATABASE_URL or DIRECT_URL in the Prisma schema so Hoista can select the migration database.",
        ));
    }
    let literal_url = regex::Regex::new(r#"(?:url|directUrl|shadowDatabaseUrl)\s*=\s*""#)
        .map_err(|_| EngineError::Store)?;
    if literal_url.is_match(schema) {
        return Err(failure(
            "Prisma database URLs must come from environment variables, not schema literals.",
        ));
    }
    let directory = root.join(&migration.migrations);
    if !directory.is_dir() {
        return Err(failure(
            "Commit Prisma migration history before deploying; Hoista never generates or resets production schema.",
        ));
    }
    let mut count = 0;
    for (visited, entry) in walkdir::WalkDir::new(&directory)
        .follow_links(false)
        .into_iter()
        .enumerate()
    {
        if visited >= MAX_HISTORY_ENTRIES {
            return Err(failure("Prisma migration history exceeds 10,000 entries."));
        }
        let entry = entry.map_err(|_| failure("Cannot read Prisma migration history."))?;
        if entry.file_type().is_symlink() {
            return Err(failure("Prisma migrations cannot be symbolic links."));
        }
        if entry.file_type().is_file() && entry.file_name() == "migration.sql" {
            count += 1;
        }
        if count > MAX_MIGRATIONS {
            return Err(failure(
                "Prisma migration history exceeds 1,000 migrations.",
            ));
        }
    }
    if count == 0 {
        return Err(failure(
            "Commit at least one Prisma migration.sql before deploying this database.",
        ));
    }
    Ok(())
}

/// The Prisma CLI in the workspace's own dependencies.
async fn cli(workspace: &Path) -> Result<PathBuf, EngineError> {
    let path = tokio::fs::canonicalize(workspace.join(CLI_ENTRY))
        .await
        .map_err(|_| failure("The locked project dependencies do not contain the Prisma CLI."))?;
    let root = tokio::fs::canonicalize(workspace)
        .await
        .map_err(|_| EngineError::Store)?;
    if !path.starts_with(root) {
        return Err(failure(
            "The Prisma CLI must belong to the isolated project dependencies.",
        ));
    }
    Ok(path)
}

/// Installs Prisma when the build hasn't, then generates its client.
pub(super) async fn prepare(plan: &Plan, workspace: &Path) -> Result<(), EngineError> {
    let Some(migration) = &plan.prisma_migration else {
        return Ok(());
    };
    validate(workspace, migration)?;
    if !workspace.join(CLI_ENTRY).is_file() {
        let manager = plan
            .package_manager
            .as_deref()
            .ok_or(failure("Prisma requires a locked Node package manager."))?;
        install_locked(manager, workspace).await?;
    }
    cli(workspace).await?;
    execute(
        plan,
        workspace,
        &Secret::new(PRISMA_GENERATE_URL.into()),
        false,
    )
    .await
}

/// Applies the checked-in migrations to `database`.
pub(super) async fn migrate(
    plan: &Plan,
    workspace: &Path,
    database: &Secret,
) -> Result<(), EngineError> {
    execute(plan, workspace, database, true)
        .await
        .map_err(|_| EngineError::Provider {
            evidence: None,
            code: "deploy.migration_failed".into(),
            message: "Prisma migration failed; new code was not released. Inspect the database migration history and resolve the failure before retrying. Database changes are never automatically reversed.".into(),
        })
}

/// Runs `prisma migrate deploy`, or `prisma generate` when `migrate` is off.
async fn execute(
    plan: &Plan,
    workspace: &Path,
    database: &Secret,
    migrate: bool,
) -> Result<(), EngineError> {
    let migration = plan
        .prisma_migration
        .as_ref()
        .ok_or(failure("No Prisma migration was planned."))?;
    validate(workspace, migration)?;
    let entry = cli(workspace).await?;
    let bytes = bounded_read(&workspace.join(CLI_PACKAGE), MAX_FILE_BYTES)?;
    let package: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| EngineError::Store)?;
    let major = package["version"]
        .as_str()
        .and_then(|version| version.split('.').next())
        .and_then(|major| major.parse::<u32>().ok())
        .ok_or(failure("Invalid Prisma version."))?;
    let mut args = vec![entry.to_str().ok_or(EngineError::Store)?];
    if migrate {
        args.extend(["migrate", "deploy"]);
    } else {
        args.push("generate");
    }
    args.extend(["--schema", &migration.schema]);
    if major == 7 {
        // Hoista's own config, for this command only: the planned database and
        // the checked-in migrations, and none of the project's seed hooks.
        let config = format!(
            "import {{ defineConfig }} from 'prisma/config';\nexport default defineConfig({{ schema: '{}', migrations: {{ path: '{}' }}, datasource: {{ url: process.env.DATABASE_URL }} }});\n",
            migration.schema, migration.migrations
        );
        tokio::fs::write(workspace.join(PRISMA_CONFIG), config)
            .await
            .map_err(|_| EngineError::Store)?;
        args.extend(["--config", PRISMA_CONFIG]);
    } else if major == 6 {
        if PRISMA_6_CONFIGS
            .iter()
            .any(|file| workspace.join(file).exists())
        {
            return Err(failure(
                "Use Prisma 7 for a custom Prisma config, or the standard Prisma 6 schema environment configuration.",
            ));
        }
    } else {
        return Err(failure(
            "Automatic migrations currently support Prisma 6 and 7.",
        ));
    }
    let env = vec![
        ("DATABASE_URL".into(), Secret::new(database.expose().into())),
        ("DIRECT_URL".into(), Secret::new(database.expose().into())),
    ];
    run(
        "node",
        &args,
        workspace,
        &env,
        STEP_TIMEOUT,
        "Prisma couldn't generate its client. Run prisma generate in the folder to see why.",
    )
    .await
}
