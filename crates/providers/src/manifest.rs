//! Adapter manifests: the parts of a provider adapter that are data, not code
//! (its API catalog, pinned CLI and the desktop app's action buttons).
//!
//! Manifests are validated on load: one that could retarget a command, pass a
//! secret as an argument or run an unverified program is refused.

use super::cli::{Command, Tool};
use crate::{
    application::ports::providers::{ProviderOperationInfo, ProviderQuickAction},
    domain::model::ResourceKind,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

const MAX_NAME_CHARS: usize = 64;
/// npm's own limit on a package name.
const MAX_NPM_NAME_CHARS: usize = 214;
/// A digest's length in base64, as integrity strings write it.
const SHA512_BASE64_CHARS: usize = 88;
const SHA256_BASE64_CHARS: usize = 44;

/// One provider's manifest, checked.
#[derive(Debug)]
pub(super) struct Manifest {
    pub id: String,
    pub api: Option<Api>,
    pub cli: Option<Tool>,
    pub actions: Vec<ProviderQuickAction>,
}

/// The provider's API as a generated catalog.
#[derive(Debug)]
pub(super) struct Api {
    /// The part of each operation's path the provider's address already has.
    pub base: String,
    pub operations: Vec<super::api_catalog::RestOperation>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    id: String,
    name: String,
    api: Option<RawApi>,
    cli: Option<RawCli>,
    #[serde(default)]
    actions: Vec<RawAction>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawApi {
    /// `rest`: operations from an OpenAPI spec.
    kind: String,
    base: String,
    /// The operations file, next to the manifest.
    operations: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCli {
    name: String,
    version: String,
    #[serde(default)]
    downloads: Vec<RawDownload>,
    #[serde(default)]
    inside: String,
    /// A CLI that runs on Node, from npm, instead of downloads.
    npm: Option<RawNpm>,
    /// A CLI that runs on Python, from PyPI; `downloads` are then uv's.
    pip: Option<RawPip>,
    /// Variables that aim every command: variable → what it takes from the
    /// resource.
    #[serde(default)]
    target_variables: BTreeMap<String, String>,
    inside_windows: Option<String>,
    #[serde(default)]
    token: String,
    /// For a token in parts: variable → how that part starts.
    #[serde(default)]
    token_parts: BTreeMap<String, String>,
    #[serde(default)]
    variables: BTreeMap<String, String>,
    #[serde(default)]
    always: Vec<String>,
    #[serde(default)]
    confirmed: Vec<String>,
    /// Options that aim a command at the resource: option → what it takes
    /// from the resource (`project`, `service`, `environment`, `app`…).
    targets: BTreeMap<String, String>,
    workdir: Option<String>,
    json_flag: Option<String>,
    yes_flag: Option<String>,
    /// The commands file, next to the manifest.
    commands: String,
    /// Positional arguments Hoista fills in from the resource, such as an
    /// edge function's name: argument → what the resource names.
    #[serde(default)]
    context: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNpm {
    package: String,
    version: String,
    bin: String,
    node: u32,
    /// The lockfile, next to the manifest.
    lockfile: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPip {
    package: String,
    version: String,
    module: String,
    python: String,
    /// The hashed requirements, next to the manifest.
    requirements: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDownload {
    platform: String,
    url: String,
    integrity: String,
    /// `tar.gz` (the default) or `zip`.
    archive: Option<String>,
    /// The binary's path in this archive, when it isn't `cli.inside`.
    inside: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAction {
    label: String,
    on: Option<ResourceKind>,
    run: String,
    description: String,
    #[serde(default)]
    args: serde_json::Map<String, Value>,
    try_first: Option<serde_json::Map<String, Value>>,
    /// Only for resources whose reference starts with one of these.
    #[serde(default)]
    refs: Vec<String>,
}

#[derive(Deserialize)]
struct RawCommands {
    cli: String,
    version: String,
    commands: Vec<Command>,
}

/// Why a manifest was refused.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(super) struct Refused(String);

fn refuse<T>(message: impl Into<String>) -> Result<T, Refused> {
    Err(Refused(message.into()))
}

const PLATFORMS: [&str; 6] = [
    "darwin-arm64",
    "darwin-x64",
    "linux-arm64",
    "linux-x64",
    "windows-arm64",
    "windows-x64",
];

fn simple_name(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_NAME_CHARS
        && text
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn variable_name(text: &str) -> bool {
    let mut bytes = text.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_uppercase() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

/// A program Hoista would run: a fixed, sane path inside the archive.
fn archive_path(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with('/')
        && text
            .split('/')
            .all(|resource| !resource.is_empty() && resource != "." && resource != "..")
}

/// An option Hoista adds itself: `--name` or `--name=value`, one argument.
pub(super) fn option(text: &str) -> bool {
    text.strip_prefix("--").is_some_and(|rest| {
        let name = rest.split('=').next().unwrap_or_default();
        simple_name(name) && !text.contains(char::is_whitespace)
    })
}

/// Loads and checks a manifest. `file` reads a file named in it, from the
/// manifest's own folder.
///
/// # Errors
/// Anything the checks below refuse, with the reason.
pub(super) fn load(text: &str, file: impl Fn(&str) -> Option<String>) -> Result<Manifest, Refused> {
    let raw: Raw =
        serde_json::from_str(text).or_else(|error| refuse(format!("not a manifest: {error}")))?;
    if !simple_name(&raw.id) {
        return refuse("id is lowercase letters, digits and dashes");
    }
    if raw.name.trim().is_empty() {
        return refuse("name is empty");
    }
    let api = raw.api.map(|api| rest(api, &file)).transpose()?;
    let cli = raw.cli.map(|cli| tool(cli, &file)).transpose()?;
    let mut actions = Vec::new();
    for action in raw.actions {
        if action.label.trim().is_empty() || action.label.ends_with('…') {
            return refuse(format!("action {:?} needs a plain label", action.label));
        }
        let known = cli.as_ref().and_then(|cli| {
            cli.commands
                .iter()
                .find(|command| command.name == action.run)
        });
        // API operations are checked by the adapter that offers them.
        if let Some(command) = known {
            for key in action
                .args
                .keys()
                .chain(action.try_first.iter().flat_map(serde_json::Map::keys))
            {
                if !command.takes(key) {
                    return refuse(format!("{} has no argument {key}", action.run));
                }
            }
        } else if let Some(op) = api
            .as_ref()
            .and_then(|api| api.operations.iter().find(|op| op.name == action.run))
        {
            for key in action
                .args
                .keys()
                .chain(action.try_first.iter().flat_map(serde_json::Map::keys))
            {
                if !op.takes(key) {
                    return refuse(format!("{} has no argument {key}", action.run));
                }
            }
        } else {
            return refuse(format!(
                "{} isn't one of the provider's operations",
                action.run
            ));
        }
        actions.push(ProviderQuickAction {
            label: action.label,
            on: action.on,
            operation: action.run,
            description: action.description,
            args: Value::Object(action.args),
            try_first: action.try_first.map(Value::Object),
            refs: action.refs,
        });
    }
    if let (Some(api), Some(cli)) = (&api, &cli)
        && let Some(both) = api
            .operations
            .iter()
            .find(|op| cli.commands.iter().any(|command| command.name == op.name))
    {
        return refuse(format!(
            "{} is both an API operation and a command",
            both.name
        ));
    }
    Ok(Manifest {
        id: raw.id,
        api,
        cli,
        actions,
    })
}

fn rest(api: RawApi, file: &impl Fn(&str) -> Option<String>) -> Result<Api, Refused> {
    if api.kind != "rest" {
        return refuse(format!("api.kind {} isn't known", api.kind));
    }
    if !api.base.is_empty()
        && (!api.base.starts_with('/') || api.base.ends_with('/') || api.base.contains(".."))
    {
        return refuse("api.base is a path such as /v1");
    }
    let Some(text) = archive_path(&api.operations)
        .then(|| file(&api.operations))
        .flatten()
    else {
        return refuse(format!("can't read {}", api.operations));
    };
    let catalog: super::api_catalog::RestCatalog =
        serde_json::from_str(&text).or_else(|error| {
            refuse(format!(
                "{} isn't an operation list: {error}",
                api.operations
            ))
        })?;
    let mut seen = std::collections::BTreeSet::new();
    for op in &catalog.operations {
        op.check()
            .or_else(|reason| refuse(format!("{}: {reason}", op.name)))?;
        if !seen.insert(op.name.clone()) {
            return refuse(format!("{} is listed twice", op.name));
        }
    }
    Ok(Api {
        base: api.base,
        operations: catalog.operations,
    })
}

/// An npm package and its lockfile: the package at exactly the CLI's
/// version, and everything it installs from the npm registry with a sha512
/// integrity, so npm installs only what was pinned.
fn package(
    npm: RawNpm,
    version: &str,
    file: &impl Fn(&str) -> Option<String>,
) -> Result<super::cli::Package, Refused> {
    let name_ok = !npm.package.is_empty()
        && npm.package.len() <= MAX_NPM_NAME_CHARS
        && npm
            .package
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-._@/".contains(&b))
        && !npm.package.contains("..");
    if !name_ok || npm.version != version {
        return refuse("npm.package and npm.version must name the CLI's exact version");
    }
    if !archive_path(&npm.bin) {
        return refuse("npm.bin is a path inside the package");
    }
    let Some(lockfile) = archive_path(&npm.lockfile)
        .then(|| file(&npm.lockfile))
        .flatten()
    else {
        return refuse(format!("can't read {}", npm.lockfile));
    };
    let lock: Value = serde_json::from_str(&lockfile)
        .or_else(|error| refuse(format!("{} isn't JSON: {error}", npm.lockfile)))?;
    if lock["lockfileVersion"].as_u64().is_none_or(|v| v < 2) {
        return refuse("the lockfile needs lockfileVersion 2 or later");
    }
    let Some(packages) = lock["packages"].as_object() else {
        return refuse("the lockfile has no packages");
    };
    let root = &packages.get("").cloned().unwrap_or_default()["dependencies"];
    if root != &serde_json::json!({ &npm.package: &npm.version }) {
        return refuse("the lockfile must depend on exactly the CLI's package and version");
    }
    let top = packages
        .get(&format!("node_modules/{}", npm.package))
        .and_then(|entry| entry["version"].as_str());
    if top != Some(npm.version.as_str()) {
        return refuse("the lockfile must install the CLI's exact version");
    }
    for (path, entry) in packages {
        if path.is_empty() {
            continue;
        }
        let resolved = entry["resolved"].as_str().unwrap_or_default();
        let integrity = entry["integrity"].as_str().unwrap_or_default();
        if !resolved.starts_with("https://registry.npmjs.org/")
            || !integrity.starts_with("sha512-")
            || entry.get("link").is_some()
        {
            return refuse(format!("{path} isn't pinned from the npm registry"));
        }
    }
    Ok(super::cli::Package {
        name: npm.package,
        version: npm.version,
        bin: npm.bin,
        node: npm.node,
        lockfile,
    })
}

/// Python requirements as `uv pip compile --generate-hashes` writes them:
/// only `name==version` lines (with markers), each with sha256 hashes, and
/// the CLI's own package at exactly its version. No other indexes, links,
/// local paths or included files.
fn python(
    pip: RawPip,
    version: &str,
    file: &impl Fn(&str) -> Option<String>,
) -> Result<super::cli::Python, Refused> {
    if !simple_name(&pip.package) || pip.version != version {
        return refuse("pip.package and pip.version must name the CLI's exact version");
    }
    if pip.module.is_empty()
        || !pip
            .module
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.')
    {
        return refuse("pip.module is a module name");
    }
    let Some((major, minor)) = pip.python.split_once('.') else {
        return refuse("pip.python is a version such as 3.12");
    };
    if major != "3" || minor.is_empty() || !minor.bytes().all(|b| b.is_ascii_digit()) {
        return refuse("pip.python is a version such as 3.12");
    }
    let Some(requirements) = archive_path(&pip.requirements)
        .then(|| file(&pip.requirements))
        .flatten()
    else {
        return refuse(format!("can't read {}", pip.requirements));
    };
    let mut logical = Vec::new();
    let mut current = String::new();
    for line in requirements.lines() {
        let line = line.trim();
        if line.starts_with('#') || (line.is_empty() && current.is_empty()) {
            continue;
        }
        let (part, more) = match line.strip_suffix('\\') {
            Some(part) => (part.trim(), true),
            None => (line, false),
        };
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(part);
        if !more {
            logical.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        logical.push(current);
    }
    let mut pinned = false;
    for entry in &logical {
        let entry = entry.split(" #").next().unwrap_or_default();
        let mut words = entry.split_whitespace();
        let first = words.next().unwrap_or_default();
        let Some((name, at)) = first.split_once("==") else {
            return refuse(format!("{first} isn't pinned with =="));
        };
        let name_ok = !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
        let at_ok = !at.is_empty()
            && at
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._+!-".contains(&b));
        if !name_ok || !at_ok {
            return refuse(format!("{first} isn't a plain pin"));
        }
        let mut hashes = 0;
        let mut in_marker = false;
        for word in words {
            if let Some(hash) = word.strip_prefix("--hash=sha256:") {
                in_marker = false;
                if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return refuse(format!("{name} has a malformed hash"));
                }
                hashes += 1;
            } else if word == ";" {
                in_marker = true;
            } else if !in_marker || word.starts_with('-') || word.contains("://") {
                return refuse(format!("{name} has more than a pin, a marker and hashes"));
            }
        }
        if hashes == 0 {
            return refuse(format!("{name} has no sha256 hash"));
        }
        if name.eq_ignore_ascii_case(&pip.package) {
            if at != pip.version {
                return refuse("the requirements must pin the CLI's exact version");
            }
            pinned = true;
        }
    }
    if !pinned {
        return refuse("the requirements must include the CLI's own package");
    }
    Ok(super::cli::Python {
        module: pip.module,
        python: pip.python,
        requirements,
    })
}

fn tool(mut cli: RawCli, file: &impl Fn(&str) -> Option<String>) -> Result<Tool, Refused> {
    if !simple_name(&cli.name) {
        return refuse("cli.name is lowercase letters, digits and dashes");
    }
    if cli.version.is_empty()
        || !cli
            .version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    {
        return refuse("cli.version is a plain version");
    }
    if cli.downloads.is_empty() == cli.npm.is_none() {
        return refuse("cli needs either downloads or npm");
    }
    if cli.pip.is_some() && cli.npm.is_some() {
        return refuse("cli is from npm or from pip, not both");
    }
    let npm = cli
        .npm
        .take()
        .map(|npm| package(npm, &cli.version, file))
        .transpose()?;
    let pip = cli
        .pip
        .take()
        .map(|pip| python(pip, &cli.version, file))
        .transpose()?;
    let mut downloads = Vec::new();
    for download in cli.downloads {
        if !PLATFORMS.contains(&download.platform.as_str()) {
            return refuse(format!("unknown platform {}", download.platform));
        }
        if downloads
            .iter()
            .any(|known: &super::cli::Download| known.platform == download.platform)
        {
            return refuse(format!("{} is listed twice", download.platform));
        }
        let Ok(url) = reqwest::Url::parse(&download.url) else {
            return refuse(format!("{} has no valid address", download.platform));
        };
        if url.scheme() != "https" || url.host_str().is_none() || !url.username().is_empty() {
            return refuse(format!("{} must download over https", download.platform));
        }
        let (digest, length) = match download.integrity.split_once('-') {
            Some(("sha512", digest)) => (digest, SHA512_BASE64_CHARS),
            Some(("sha256", digest)) => (digest, SHA256_BASE64_CHARS),
            _ => ("", 1),
        };
        if digest.len() != length
            || !digest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=')
        {
            return refuse(format!(
                "{} needs a sha512 or sha256 integrity",
                download.platform
            ));
        }
        let archive = download.archive.unwrap_or_else(|| "tar.gz".into());
        if archive != "tar.gz" && archive != "zip" {
            return refuse(format!("{} has an unknown archive kind", download.platform));
        }
        let inside = download.inside.unwrap_or_else(|| {
            if download.platform.starts_with("windows") {
                cli.inside_windows
                    .clone()
                    .unwrap_or_else(|| cli.inside.clone())
            } else {
                cli.inside.clone()
            }
        });
        if !archive_path(&inside) {
            return refuse(format!(
                "{}'s program is a path inside the archive",
                download.platform
            ));
        }
        downloads.push(super::cli::Download {
            platform: download.platform,
            url: download.url,
            integrity: download.integrity,
            archive,
            inside,
        });
    }
    if (!downloads.is_empty() && !archive_path(&cli.inside))
        || cli
            .inside_windows
            .as_deref()
            .is_some_and(|path| !archive_path(path))
    {
        return refuse("cli.inside is a path inside the archive");
    }
    if cli.token_parts.is_empty() == cli.token.is_empty() {
        return refuse("cli needs either token or token_parts");
    }
    if !cli.token.is_empty() && !variable_name(&cli.token) {
        return refuse("cli.token is a variable name");
    }
    for (variable, start) in &cli.token_parts {
        let start_ok = !start.is_empty()
            && start
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !variable_name(variable) || !start_ok {
            return refuse(format!("token part {variable} isn't allowed"));
        }
    }
    for (variable, from) in &cli.target_variables {
        if !variable_name(variable)
            || variable == &cli.token
            || cli.token_parts.contains_key(variable)
            || !simple_name(from)
        {
            return refuse(format!("target variable {variable} isn't allowed"));
        }
    }
    for (key, value) in &cli.variables {
        if !variable_name(key)
            || key == &cli.token
            || cli.token_parts.contains_key(key)
            || value.contains(['\n', '\0'])
        {
            return refuse(format!("variable {key} isn't allowed"));
        }
    }
    if let Some(bad) = cli
        .always
        .iter()
        .chain(&cli.confirmed)
        .find(|arg| !option(arg))
    {
        return refuse(format!("{bad} isn't one option"));
    }
    // Each target is one option, `--name=<value from the resource>`; a CLI
    // aimed by variables may have none.
    if cli.targets.is_empty() && cli.target_variables.is_empty() {
        return refuse("cli.targets is empty");
    }
    for (flag, from) in &cli.targets {
        if !simple_name(flag) || !simple_name(from) {
            return refuse(format!("target {flag} isn't allowed"));
        }
    }
    if cli
        .workdir
        .as_ref()
        .is_some_and(|workdir| !workdir.ends_with('=') || !option(&format!("{workdir}x")))
    {
        return refuse("cli.workdir is one option such as --workdir=");
    }
    for flag in cli.json_flag.iter().chain(&cli.yes_flag) {
        if !simple_name(flag) {
            return refuse(format!("{flag} isn't an option name"));
        }
    }
    let commands_text = if archive_path(&cli.commands) {
        file(&cli.commands)
    } else {
        None
    };
    let Some(commands_text) = commands_text else {
        return refuse(format!("can't read {}", cli.commands));
    };
    let catalog: RawCommands = serde_json::from_str(&commands_text)
        .or_else(|error| refuse(format!("{} isn't a command list: {error}", cli.commands)))?;
    // Generated from this exact binary, or it may not match what runs.
    if catalog.cli != cli.name || catalog.version != cli.version {
        return refuse(format!(
            "{} was made for {} {}, not {} {}",
            cli.commands, catalog.cli, catalog.version, cli.name, cli.version
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for command in &catalog.commands {
        command
            .check(&cli.name, &cli.targets)
            .or_else(|reason| refuse(format!("{}: {reason}", command.name)))?;
        if cli.target_variables.is_empty() && !command.aimed() {
            return refuse(format!("{} isn't aimed at a resource", command.name));
        }
        if !seen.insert(command.name.clone()) {
            return refuse(format!("{} is listed twice", command.name));
        }
    }
    let mut context = BTreeMap::new();
    for (arg, from) in cli.context {
        if !simple_name(&arg.replace('_', "-")) || !simple_name(&from) {
            return refuse(format!("context {arg} isn't allowed"));
        }
        context.insert(arg, from);
    }
    Ok(Tool {
        name: cli.name,
        version: cli.version,
        downloads,
        npm,
        pip,
        target_variables: cli.target_variables,
        token_variable: cli.token,
        token_parts: cli.token_parts.into_iter().collect(),
        variables: cli.variables.into_iter().collect(),
        always: cli.always,
        confirmed: cli.confirmed,
        targets: cli.targets,
        workdir: cli.workdir,
        json_flag: cli.json_flag,
        yes_flag: cli.yes_flag,
        context,
        commands: catalog.commands,
    })
}

/// The manifest built into Hoista for provider `id`, loaded and checked once.
/// One that doesn't load offers nothing, and says why in the log; the
/// tests load each, so a bad one never ships.
pub(super) fn builtin(id: &str) -> &'static Manifest {
    use std::sync::OnceLock;
    static SUPABASE: OnceLock<Manifest> = OnceLock::new();
    static RAILWAY: OnceLock<Manifest> = OnceLock::new();
    static FLY: OnceLock<Manifest> = OnceLock::new();
    static WRANGLER: OnceLock<Manifest> = OnceLock::new();
    static VERCEL: OnceLock<Manifest> = OnceLock::new();
    static NEON: OnceLock<Manifest> = OnceLock::new();
    static RENDER: OnceLock<Manifest> = OnceLock::new();
    static RESEND: OnceLock<Manifest> = OnceLock::new();
    static RUNPOD: OnceLock<Manifest> = OnceLock::new();
    static MODAL: OnceLock<Manifest> = OnceLock::new();
    static NONE: OnceLock<Manifest> = OnceLock::new();
    let (cell, text, files): (_, &str, &[(&str, &str)]) = match id {
        "supabase" => (
            &SUPABASE,
            include_str!("supabase/manifest.json"),
            &[
                ("api_ops.json", include_str!("supabase/api_ops.json")),
                ("cli.json", include_str!("supabase/cli.json")),
            ],
        ),
        "railway" => (
            &RAILWAY,
            include_str!("railway/manifest.json"),
            &[("cli.json", include_str!("railway/cli.json"))],
        ),
        "fly" => (
            &FLY,
            include_str!("fly/manifest.json"),
            &[
                ("api_ops.json", include_str!("fly/api_ops.json")),
                ("cli.json", include_str!("fly/cli.json")),
            ],
        ),
        "wrangler" => (
            &WRANGLER,
            include_str!("wrangler/manifest.json"),
            &[
                ("api_ops.json", include_str!("wrangler/api_ops.json")),
                ("cli.json", include_str!("wrangler/cli.json")),
                (
                    "package-lock.json",
                    include_str!("wrangler/package-lock.json"),
                ),
            ],
        ),
        "vercel" => (
            &VERCEL,
            include_str!("vercel/manifest.json"),
            &[
                ("api_ops.json", include_str!("vercel/api_ops.json")),
                ("cli.json", include_str!("vercel/cli.json")),
                (
                    "package-lock.json",
                    include_str!("vercel/package-lock.json"),
                ),
            ],
        ),
        "neon" => (
            &NEON,
            include_str!("neon/manifest.json"),
            &[
                ("api_ops.json", include_str!("neon/api_ops.json")),
                ("cli.json", include_str!("neon/cli.json")),
                ("package-lock.json", include_str!("neon/package-lock.json")),
            ],
        ),
        "render" => (
            &RENDER,
            include_str!("render/manifest.json"),
            &[
                ("api_ops.json", include_str!("render/api_ops.json")),
                ("cli.json", include_str!("render/cli.json")),
            ],
        ),
        "resend" => (
            &RESEND,
            include_str!("resend/manifest.json"),
            &[("api_ops.json", include_str!("resend/api_ops.json"))],
        ),
        "runpod" => (
            &RUNPOD,
            include_str!("runpod/manifest.json"),
            &[
                ("api_ops.json", include_str!("runpod/api_ops.json")),
                ("cli.json", include_str!("runpod/cli.json")),
                ("requirements.txt", include_str!("runpod/requirements.txt")),
            ],
        ),
        "modal" => (
            &MODAL,
            include_str!("modal/manifest.json"),
            &[
                ("cli.json", include_str!("modal/cli.json")),
                ("requirements.txt", include_str!("modal/requirements.txt")),
            ],
        ),
        _ => (&NONE, "", &[]),
    };
    cell.get_or_init(|| {
        load_builtin(id, text, files).unwrap_or_else(|refused| Manifest::empty(&refused))
    })
}

fn load_builtin(id: &str, text: &str, files: &[(&str, &str)]) -> Result<Manifest, Refused> {
    load(text, |name| {
        files
            .iter()
            .find(|(file, _)| *file == name)
            .map(|(_, body)| (*body).to_owned())
    })
    .and_then(|manifest| manifest.named(id))
}

impl Manifest {
    /// This manifest, if it's for the provider `id`.
    ///
    /// # Errors
    /// A manifest for another provider.
    pub(super) fn named(self, id: &str) -> Result<Self, Refused> {
        if self.id == id {
            Ok(self)
        } else {
            refuse(format!("expected the {id} manifest, got {}", self.id))
        }
    }

    /// What a manifest that didn't load leaves: nothing offered. Logged once.
    pub(super) fn empty(refused: &Refused) -> Self {
        tracing::error!(reason = %refused, "a built-in adapter manifest was refused");
        Self {
            id: String::new(),
            api: None,
            cli: None,
            actions: Vec::new(),
        }
    }

    /// Everything the provider offers: API operations, then CLI commands.
    pub(super) fn operations(&self) -> Vec<ProviderOperationInfo> {
        let mut all = self
            .api
            .as_ref()
            .map(|api| super::api_catalog::rest_operations(&api.operations))
            .unwrap_or_default();
        all.extend(
            self.cli
                .as_ref()
                .map(super::cli::operations)
                .unwrap_or_default(),
        );
        all
    }

    /// The API operation named `name`.
    pub(super) fn api_operation(&self, name: &str) -> Option<&super::api_catalog::RestOperation> {
        self.api
            .as_ref()?
            .operations
            .iter()
            .find(|op| op.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn supabase() -> (String, impl Fn(&str) -> Option<String>) {
        (
            include_str!("supabase/manifest.json").to_owned(),
            |name: &str| match name {
                "cli.json" => Some(include_str!("supabase/cli.json").to_owned()),
                "api_ops.json" => Some(include_str!("supabase/api_ops.json").to_owned()),
                _ => None,
            },
        )
    }

    fn changed(edit: impl Fn(&mut Value)) -> String {
        let mut value: Value =
            serde_json::from_str(include_str!("supabase/manifest.json")).unwrap();
        edit(&mut value);
        value.to_string()
    }

    #[test]
    fn python_requirements_are_only_hashed_pins() {
        let text = include_str!("modal/manifest.json");
        let with = |requirements: &'static str| {
            load(text, move |name| match name {
                "cli.json" => Some(include_str!("modal/cli.json").to_owned()),
                "requirements.txt" => Some(requirements.to_owned()),
                _ => None,
            })
        };
        assert!(with(include_str!("modal/requirements.txt")).is_ok());
        let hash = "--hash=sha256:065665c041c42a5938ed220bdcd7230f22527fbec085e1853d2402c8a3615d9d";
        let good = format!(
            "modal==1.6.3 \\\n    {hash}\ncolorama==0.4.6 ; sys_platform == 'win32' \\\n    {hash}\n"
        );
        assert!(with(Box::leak(good.into_boxed_str())).is_ok());
        for bad in [
            "modal==1.6.3\n".to_owned(),
            format!("modal>=1.6 {hash}\n"),
            format!("modal==1.6.2 {hash}\n"),
            format!("modal==1.6.3 {hash}\n--extra-index-url https://example.com {hash}\n"),
            format!("modal==1.6.3 {hash}\nevil @ https://example.com/evil.whl {hash}\n"),
            format!("grpclib==0.4.8 {hash}\n"),
        ] {
            assert!(
                with(Box::leak(bad.clone().into_boxed_str())).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn every_built_in_manifest_loads() {
        for id in [
            "supabase", "railway", "fly", "wrangler", "vercel", "neon", "render", "resend", "modal",
        ] {
            let manifest = builtin(id);
            if manifest.id != id {
                let files: Vec<(&str, String)> = [
                    "cli.json",
                    "api_ops.json",
                    "package-lock.json",
                    "requirements.txt",
                ]
                .iter()
                .filter_map(|file| {
                    std::fs::read_to_string(format!("src/{id}/{file}"))
                        .ok()
                        .map(|text| (*file, text))
                })
                .collect();
                let text = std::fs::read_to_string(format!("src/{id}/manifest.json")).unwrap();
                let refs: Vec<(&str, &str)> = files
                    .iter()
                    .map(|(file, text)| (*file, text.as_str()))
                    .collect();
                panic!("{id}: {:?}", load_builtin(id, &text, &refs).err());
            }
            // Each has a full command line or a full API, or both.
            assert!(
                manifest
                    .cli
                    .as_ref()
                    .is_some_and(|cli| cli.commands.len() > 50)
                    || manifest
                        .api
                        .as_ref()
                        .is_some_and(|api| api.operations.len() > 50)
            );
            assert!(!manifest.actions.is_empty());
        }
    }

    #[test]
    fn the_supabase_manifest_loads() {
        let (text, file) = supabase();
        let manifest = load(&text, file).expect("manifest");
        assert_eq!(manifest.id, "supabase");
        let cli = manifest.cli.as_ref().unwrap();
        assert_eq!(cli.version, "3.0.0");
        assert!(cli.commands.len() > 60);
        assert!(
            manifest
                .actions
                .iter()
                .any(|action| action.label == "Apply migrations")
        );
        let all = manifest.operations();
        assert!(all.iter().any(|op| op.name == "supabase db push"));
        assert!(all.iter().any(|op| op.name == "getProject"));
    }

    #[test]
    fn manifests_that_could_escape_are_refused() {
        let (_, file) = supabase();
        for (why, text) in [
            (
                "http download",
                changed(|m| m["cli"]["downloads"][0]["url"] = "http://example.com/x.tgz".into()),
            ),
            (
                "no checksum",
                changed(|m| m["cli"]["downloads"][0]["integrity"] = "sha1-abc".into()),
            ),
            (
                "unknown platform",
                changed(|m| m["cli"]["downloads"][0]["platform"] = "plan9-x64".into()),
            ),
            (
                "target as its own argument",
                changed(|m| m["cli"]["targets"] = serde_json::json!({"remote x": "project"})),
            ),
            (
                "target split in two",
                changed(|m| m["cli"]["targets"] = serde_json::json!({})),
            ),
            (
                "program outside the archive",
                changed(|m| m["cli"]["inside"] = "../../bin/sh".into()),
            ),
            (
                "token variable overwritten",
                changed(|m| m["cli"]["variables"]["SUPABASE_ACCESS_TOKEN"] = "x".into()),
            ),
            (
                "an extra argument that isn't an option",
                changed(|m| m["cli"]["always"] = serde_json::json!(["rm"])),
            ),
            (
                "commands file outside",
                changed(|m| m["cli"]["commands"] = "../../secrets.json".into()),
            ),
            ("unknown field", changed(|m| m["shell"] = "bash".into())),
            (
                "action with a missing argument",
                changed(|m| m["actions"][0]["args"] = serde_json::json!({"password": "x"})),
            ),
            (
                "action with dots",
                changed(|m| m["actions"][0]["label"] = "Apply…".into()),
            ),
            (
                "version mismatch",
                changed(|m| m["cli"]["version"] = "2.0.0".into()),
            ),
            (
                "unknown api kind",
                changed(|m| m["api"]["kind"] = "soap".into()),
            ),
            (
                "action for nothing",
                changed(|m| m["actions"][0]["run"] = "dropEverything".into()),
            ),
        ] {
            assert!(load(&text, &file).is_err(), "{why} must be refused");
        }
    }

    #[test]
    fn commands_that_take_secrets_or_another_target_are_refused() {
        let (text, _) = supabase();
        let mut commands: Value = serde_json::from_str(include_str!("supabase/cli.json")).unwrap();
        commands["commands"][0]["flags"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "name": "db-password",
                "kind": "string",
                "choices": [],
                "path": false,
                "description": "",
            }));
        let edited = commands.to_string();
        assert!(
            load(&text, |name| (name == "cli.json")
                .then(|| edited.clone())
                .or_else(|| Some(
                    include_str!("supabase/api_ops.json").to_owned()
                )))
            .is_err()
        );
        let mut commands: Value = serde_json::from_str(include_str!("supabase/cli.json")).unwrap();
        commands["commands"][0]["flags"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"name": "remote", "kind": "string", "choices": [], "path": false, "description": ""}));
        let edited = commands.to_string();
        assert!(
            load(&text, |name| (name == "cli.json")
                .then(|| edited.clone())
                .or_else(|| Some(
                    include_str!("supabase/api_ops.json").to_owned()
                )))
            .is_err()
        );
        let mut commands: Value = serde_json::from_str(include_str!("supabase/cli.json")).unwrap();
        commands["commands"][0]["words"] = serde_json::json!(["--config", "x"]);
        let edited = commands.to_string();
        assert!(
            load(&text, |name| (name == "cli.json")
                .then(|| edited.clone())
                .or_else(|| Some(
                    include_str!("supabase/api_ops.json").to_owned()
                )))
            .is_err()
        );
    }
}
