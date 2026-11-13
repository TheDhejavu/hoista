//! Runs a provider's own CLI on the person's behalf, so an agent can use it
//! without ever holding the person's token.
//!
//! Each run gets a pinned, checksummed binary, no shell, a cleared environment
//! and its own home folder, so it never touches the person's own CLI sign-in.

use super::http::ProviderHttp;
use crate::{
    application::ports::{
        catalog::EngineError,
        providers::{ProviderOperationArg, ProviderOperationInfo, ProviderRisk},
    },
    domain::{Secret, leaks::LeakScanner},
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::{
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};

/// How long an install step (npm or uv) may run.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const VERSION_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_VERSION_BYTES: u64 = 64;

/// Downloading a pinned build: its size limit, and how long it may take.
const MAX_DOWNLOAD: u64 = 400 * 1024 * 1024;
const MAX_REDIRECTS: usize = 3;
const DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// How long a provider's command may run.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// How long a run waits for a first install before saying it's under way.
const INSTALL_WAIT: Duration = Duration::from_secs(20);

/// The most of a command's output that's read, and of its error text that's shown.
const MAX_OUTPUT: u64 = 4 * 1024 * 1024;
const MAX_MESSAGE_CHARS: usize = 4000;
/// Lines of detail kept after a CLI's error line.
const MAX_DETAIL_LINES: usize = 3;

/// One provider CLI Hoista can run, from the provider's manifest (checked by
/// `manifest.rs` when it loads).
#[derive(Debug)]
pub(super) struct Tool {
    /// The binary's name, also its folder under `tools/`.
    pub name: String,
    pub version: String,
    /// Where each platform's build comes from and its checksum; empty for
    /// a CLI that comes from npm.
    pub downloads: Vec<Download>,
    /// A CLI that runs on Node: its npm package, installed from a pinned
    /// lockfile and run with the computer's own Node.
    pub npm: Option<Package>,
    /// A CLI that runs on Python: its package, installed from pinned,
    /// hashed requirements into Hoista's own Python. `downloads` are then
    /// the pinned uv that sets it up.
    pub pip: Option<Python>,
    /// Variables that aim every command at the resource Hoista chose, and
    /// what each takes from it: `CLOUDFLARE_ACCOUNT_ID` ← `account`.
    pub target_variables: std::collections::BTreeMap<String, String>,
    /// The variable the provider token goes in.
    pub token_variable: String,
    /// For a token in parts, the variable each part goes in and how that
    /// part starts: `MODAL_TOKEN_ID` ← `ak-`.
    pub token_parts: Vec<(String, String)>,
    /// Fixed variables: telemetry, update checks and keychains off.
    pub variables: Vec<(String, String)>,
    /// Arguments every run gets: machine-readable output, no prompts.
    pub always: Vec<String>,
    /// Arguments a change gets once it's allowed (the CLI's own "yes").
    pub confirmed: Vec<String>,
    /// The options that aim a command at the resource Hoista chose, and
    /// what each takes from it: `remote` ← `project` for Supabase.
    pub targets: std::collections::BTreeMap<String, String>,
    /// The option that names the working folder, such as `--workdir=`; the
    /// folder is also where the CLI runs.
    pub workdir: Option<String>,
    /// The CLI's machine-readable output option, on commands that have it.
    pub json_flag: Option<String>,
    /// The CLI's own "yes" option, added to allowed changes that have it.
    pub yes_flag: Option<String>,
    /// Arguments filled in from the resource: argument → what it names.
    pub context: std::collections::BTreeMap<String, String>,
    /// The commands, generated from this binary's help.
    pub commands: Vec<Command>,
}

/// An npm package and the lockfile that pins it and everything it uses.
#[derive(Debug, Clone)]
pub(super) struct Package {
    pub name: String,
    pub version: String,
    /// Its program, inside the package: `bin/wrangler.js`.
    pub bin: String,
    /// The oldest Node major version it runs on.
    pub node: u32,
    /// `package-lock.json`, checked: every package from the npm registry,
    /// each with its sha512 integrity.
    pub lockfile: String,
}

/// A Python package and the requirements that pin it and everything it
/// uses, each with its sha256.
#[derive(Debug, Clone)]
pub(super) struct Python {
    /// Its module, run as `python -m <module>`.
    pub module: String,
    /// The Python version it runs on, such as `3.12`; uv installs its own.
    pub python: String,
    /// `requirements.txt`, checked: every package pinned with its hashes.
    pub requirements: String,
}

/// What to start for a run: a program, and the arguments before the
/// command's own (a Node CLI's script).
#[derive(Clone)]
struct Program {
    path: PathBuf,
    prefix: Vec<PathBuf>,
}

/// A pinned build for one platform: a gzipped tar or a zip, with an
/// npm-style `sha512-<base64>` or `sha256-<base64>` integrity.
#[derive(Debug, Clone)]
pub(super) struct Download {
    /// `darwin-arm64`, `linux-x64`, `windows-x64`…
    pub platform: String,
    pub url: String,
    pub integrity: String,
    /// `tar.gz` or `zip`.
    pub archive: String,
    /// The binary's path inside this archive.
    pub inside: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Command {
    pub name: String,
    words: Vec<String>,
    area: String,
    risk: ProviderRisk,
    description: String,
    folder: bool,
    dry_run: bool,
    /// Which of the CLI's target options this command takes.
    targets: Vec<String>,
    /// Whether it takes the CLI's machine-readable output option.
    json: bool,
    /// Whether it takes the CLI's "yes" option.
    yes: bool,
    /// Options it always runs with, such as `--no-tail` for logs that would
    /// otherwise stream forever.
    fixed: Vec<String>,
    args: Vec<Positional>,
    flags: Vec<Flag>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Positional {
    name: String,
    many: bool,
    kind: String,
    required: bool,
    path: bool,
    description: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Flag {
    name: String,
    kind: String,
    choices: Vec<String>,
    many: bool,
    path: bool,
    description: String,
}

const KINDS: [&str; 4] = ["bool", "string", "integer", "choice"];

impl Command {
    /// Whether one of the CLI's target options aims it.
    pub(super) fn aimed(&self) -> bool {
        !self.targets.is_empty()
    }

    /// Whether the command takes this argument.
    pub(super) fn takes(&self, key: &str) -> bool {
        self.args.iter().any(|arg| arg.name == key)
            || self.flags.iter().any(|flag| flag.name == key)
    }

    /// Refuses a command that could be aimed elsewhere, take a secret, or
    /// read as something other than the CLI's own command.
    pub(super) fn check(
        &self,
        cli: &str,
        targets: &std::collections::BTreeMap<String, String>,
    ) -> Result<(), &'static str> {
        if self.targets.iter().any(|flag| !targets.contains_key(flag)) {
            return Err("it isn't aimed at a resource Hoista chooses");
        }
        if self.fixed.iter().any(|arg| !super::manifest::option(arg)) {
            return Err("a fixed argument isn't one option");
        }
        if self.name != format!("{cli} {}", self.words.join(" ")) {
            return Err("its name must be the CLI and its words");
        }
        if self.words.is_empty()
            || self.words.iter().any(|word| {
                word.is_empty()
                    || word.starts_with('-')
                    || !word
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            })
        {
            return Err("its words must be plain subcommands");
        }
        for flag in &self.flags {
            if flag.name.is_empty()
                || !flag
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                || flag.name.starts_with('-')
            {
                return Err("a flag has an unusual name");
            }
            if targets.contains_key(&flag.name) {
                return Err("it lets callers choose the target");
            }
            if !KINDS.contains(&flag.kind.as_str()) {
                return Err("a flag has an unknown kind");
            }
            if flag.kind == "choice" && flag.choices.is_empty() {
                return Err("a choice flag has no choices");
            }
        }
        for name in self
            .flags
            .iter()
            .map(|flag| flag.name.as_str())
            .chain(self.args.iter().map(|arg| arg.name.as_str()))
        {
            if super::api_catalog::secret_key(name) || ["db-url", "url", "dsn"].contains(&name) {
                return Err("it takes a secret or a database address");
            }
        }
        if self
            .args
            .iter()
            .any(|arg| !["string", "integer"].contains(&arg.kind.as_str()))
        {
            return Err("an argument has an unknown kind");
        }
        Ok(())
    }
}

/// Hoista's own pinned, checked copy of a downloaded CLI, installing it the
/// first time, for an adapter that runs the binary itself.
///
/// # Errors
/// A tool Hoista couldn't get, or one that's still being set up.
pub(super) async fn binary(tool: &Tool, http: &ProviderHttp) -> Result<PathBuf, EngineError> {
    let tools = http.tools().ok_or(EngineError::Store)?;
    let program = program(tool, tools).await?;
    if !program.prefix.is_empty() {
        return Err(EngineError::Store);
    }
    Ok(program.path)
}

/// Whether `name` is one of this CLI's commands.
pub(super) fn has(tool: &Tool, name: &str) -> bool {
    tool.commands.iter().any(|command| command.name == name)
}

/// The CLI's commands as provider operations.
pub(super) fn operations(tool: &Tool) -> Vec<ProviderOperationInfo> {
    tool.commands
        .iter()
        .map(|command| {
            let mut description = command.description.clone();
            if command.folder {
                description.push_str(" Works on the app's linked source folder.");
            }
            if command.dry_run {
                description.push_str(" Try it first with dry-run.");
            }
            let mut args: Vec<ProviderOperationArg> = command
                .args
                .iter()
                .map(|arg| ProviderOperationArg {
                    name: arg.name.clone(),
                    required: arg.required && !tool.context.contains_key(&arg.name),
                    filled_in: tool.context.get(&arg.name).cloned(),
                    schema: schema(&arg.kind, &[], arg.many, &arg.description, arg.path),
                })
                .collect();
            args.extend(command.flags.iter().map(|flag| ProviderOperationArg {
                name: flag.name.clone(),
                required: false,
                filled_in: None,
                schema: schema(
                    &flag.kind,
                    &flag.choices,
                    flag.many,
                    &flag.description,
                    flag.path,
                ),
            }));
            ProviderOperationInfo {
                name: command.name.clone(),
                area: format!("{} (command line)", command.area),
                risk: command.risk,
                request: "Command".into(),
                description,
                args,
            }
        })
        .collect()
}

fn schema(kind: &str, choices: &[String], many: bool, description: &str, path: bool) -> Value {
    let description = if path {
        format!("{description} A path inside the app's folder.")
    } else {
        description.to_owned()
    };
    let one = match kind {
        "bool" => return json!({"type": "boolean", "description": description}),
        "integer" => json!({"type": "integer"}),
        "choice" => json!({"type": "string", "enum": choices}),
        _ => json!({"type": "string"}),
    };
    if many {
        json!({"description": description, "anyOf": [one, {"type": "array", "items": one}]})
    } else {
        let mut one = one;
        one["description"] = Value::String(description);
        one
    }
}

/// The platform's build name, as npm-style packages spell it.
fn platform() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("linux", "x86_64") => "linux-x64",
        ("windows", "aarch64") => "windows-arm64",
        ("windows", "x86_64") => "windows-x64",
        _ => return None,
    })
}

fn missing(message: &'static str) -> EngineError {
    EngineError::Provider {
        evidence: None,
        code: "provider.tool_unavailable".into(),
        message: message.into(),
    }
}

static INSTALLING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Starts installing `tool` in the background, if it isn't already; one
/// install serves every later run.
pub(super) fn prepare(tool: &'static Tool, http: &ProviderHttp) {
    let Some(tools) = http.tools().map(Path::to_path_buf) else {
        return;
    };
    tokio::spawn(async move {
        let _ = program(tool, &tools).await;
    });
}

/// What an install came to, kept for every run waiting on it.
type Installed = Option<Result<Program, (String, String)>>;

/// Installs in progress, by folder, so one install serves every run that
/// asks for it while it's going.
static IN_FLIGHT: std::sync::Mutex<
    std::collections::BTreeMap<PathBuf, tokio::sync::watch::Receiver<Installed>>,
> = std::sync::Mutex::new(std::collections::BTreeMap::new());

/// What to run: the verified binary, or Node and the installed package.
///
/// The install runs on its own, never tied to the request that started it:
/// a caller that gives up (an agent's call timing out) doesn't stop it, and
/// a first install that takes a while (Wrangler downloads about 100 MB)
/// finishes once for everyone. A run waits a little, then says the tool is
/// still being set up.
async fn program(tool: &Tool, tools: &Path) -> Result<Program, EngineError> {
    let folder = tools.join(&tool.name).join(&tool.version);
    let mut answer = {
        let mut flights = IN_FLIGHT.lock().map_err(|_| EngineError::Store)?;
        if let Some(found) = flights.get(&folder) {
            found.clone()
        } else {
            let (sender, receiver) = tokio::sync::watch::channel(None);
            let setup = Tool {
                name: tool.name.clone(),
                version: tool.version.clone(),
                downloads: tool.downloads.clone(),
                npm: tool.npm.clone(),
                pip: tool.pip.clone(),
                target_variables: std::collections::BTreeMap::new(),
                token_variable: String::new(),
                token_parts: Vec::new(),
                variables: Vec::new(),
                always: Vec::new(),
                confirmed: Vec::new(),
                targets: std::collections::BTreeMap::new(),
                workdir: None,
                json_flag: None,
                yes_flag: None,
                context: std::collections::BTreeMap::new(),
                commands: Vec::new(),
            };
            let tools = tools.to_path_buf();
            let key = folder.clone();
            tokio::spawn(async move {
                let result = match (&setup.npm, &setup.pip) {
                    (Some(package), _) => installed_package(&setup, package, &tools).await,
                    (None, Some(pip)) => installed_python(&setup, pip, &tools).await,
                    (None, None) => installed(&setup, &tools).await.map(|path| Program {
                        path,
                        prefix: Vec::new(),
                    }),
                };
                let result = result.map_err(|error| match error {
                    EngineError::Provider { code, message, .. } => (code, message),
                    _ => (
                        "provider.tool_unavailable".to_owned(),
                        "Hoista couldn't set up this provider's command-line tool. Try again."
                            .to_owned(),
                    ),
                });
                if let Ok(mut flights) = IN_FLIGHT.lock() {
                    flights.remove(&key);
                }
                sender.send_replace(Some(result));
            });
            flights.insert(folder, receiver.clone());
            receiver
        }
    };
    let done = tokio::time::timeout(INSTALL_WAIT, answer.wait_for(Option::is_some)).await;
    match done {
        Ok(Ok(found)) => match found.clone() {
            Some(Ok(program)) => Ok(program),
            Some(Err((code, message))) => Err(EngineError::Provider {
                evidence: None,
                code,
                message,
            }),
            None => Err(EngineError::Store),
        },
        Ok(Err(_)) => Err(EngineError::Store),
        Err(_) => Err(EngineError::Provider {
            evidence: None,
            code: "provider.tool_installing".into(),
            message: format!(
                "Hoista is setting up {} {} for first use; it downloads once and is kept. Try again in a minute.",
                tool.name, tool.version
            ),
        }),
    }
}

/// The computer's Node, if it's new enough.
async fn node(at_least: u32) -> Result<PathBuf, EngineError> {
    let words = || EngineError::Provider {
        evidence: None,
        code: "provider.tool_unavailable".into(),
        message: format!(
            "This provider's command-line tool runs on Node.js {at_least} or later. Install it, then run `hoist tools configure` so Hoista finds it."
        ),
    };
    let node = hoista_system::tool_paths::executable("node").ok_or_else(words)?;
    let mut ask = tokio::process::Command::new(&node);
    ask.arg("--version").env_clear().stdin(Stdio::null());
    hoista_system::process::inherit_environment(&mut ask);
    let (status, out) =
        hoista_system::process::capture(&mut ask, MAX_VERSION_BYTES, VERSION_TIMEOUT)
            .await
            .map_err(|_| words())?;
    let major = String::from_utf8_lossy(&out)
        .trim()
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|major| major.parse::<u32>().ok());
    if !status.success() || major.is_none_or(|major| major < at_least) {
        return Err(words());
    }
    Ok(node)
}

/// A Node CLI, installed from its pinned lockfile the first time: npm
/// checks every package against the lockfile's integrity, and no install
/// scripts run.
async fn installed_package(
    tool: &Tool,
    package: &Package,
    tools: &Path,
) -> Result<Program, EngineError> {
    use sha2::{Digest, Sha256};
    let node = node(package.node).await?;
    let folder = tools.join(&tool.name).join(&tool.version);
    let script = folder
        .join("node_modules")
        .join(&package.name)
        .join(&package.bin);
    let lock: String = Sha256::digest(package.lockfile.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let marker = folder.join(".verified");
    let _held = INSTALLING.lock().await;
    if std::fs::read_to_string(&marker).is_ok_and(|text| text == lock) && script.is_file() {
        return Ok(Program {
            path: node,
            prefix: vec![script],
        });
    }
    for level in [tools, &tools.join(&tool.name), &folder] {
        hoista_system::paths::private_directory(level).map_err(|_| EngineError::Store)?;
    }
    let manifest = json!({
        "name": format!("hoista-{}", tool.name),
        "private": true,
        "dependencies": {package.name.clone(): package.version.clone()},
    });
    std::fs::write(folder.join("package.json"), manifest.to_string())
        .and_then(|()| std::fs::write(folder.join("package-lock.json"), &package.lockfile))
        .map_err(|_| EngineError::Store)?;
    let npm = hoista_system::tool_paths::executable("npm").ok_or(missing(
        "This provider's command-line tool installs with npm, which comes with Node.js. Install Node.js, then run `hoist tools configure`.",
    ))?;
    let home = tools.join(&tool.name).join("home");
    hoista_system::paths::private_directory(&home).map_err(|_| EngineError::Store)?;
    let mut install = tokio::process::Command::new(&npm);
    install
        .args([
            "ci",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
            "--omit=dev",
            "--loglevel=error",
        ])
        .env_clear()
        .current_dir(&folder)
        .stdin(Stdio::null());
    hoista_system::process::inherit_environment(&mut install);
    install
        .env("HOME", &home)
        .env("npm_config_cache", tools.join("npm-cache"))
        .env("npm_config_update_notifier", "false")
        .env("CI", "1");
    let (status, _, errors) = hoista_system::process::capture_all(
        &mut install,
        None,
        MAX_OUTPUT,
        INSTALL_TIMEOUT,
    )
    .await
    .map_err(|_| missing("Hoista couldn't install this provider's command-line tool. Check the connection and try again."))?;
    if !status.success() || !script.is_file() {
        let said = String::from_utf8_lossy(&errors)
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or_default()
            .to_owned();
        return Err(EngineError::Provider {
            evidence: None,
            code: "provider.tool_unavailable".into(),
            message: format!(
                "Hoista couldn't install {} {}: {said}",
                package.name, package.version
            ),
        });
    }
    std::fs::write(&marker, &lock).map_err(|_| EngineError::Store)?;
    Ok(Program {
        path: node,
        prefix: vec![script],
    })
}

/// A Python CLI, installed the first time: Hoista's pinned uv sets up its
/// own Python (checked against uv's own list of checksums) in the tools
/// folder, then installs only the pinned wheels, each checked against its
/// hash, so no package's build code runs.
async fn installed_python(tool: &Tool, pip: &Python, tools: &Path) -> Result<Program, EngineError> {
    use sha2::{Digest, Sha256};
    let uv = installed(tool, tools).await?;
    let folder = tools.join(&tool.name).join(&tool.version);
    let venv = folder.join("venv");
    let python = if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python")
    };
    let program = Program {
        path: python.clone(),
        // Isolated: the person's own Python settings and packages stay out.
        prefix: vec!["-I".into(), "-m".into(), PathBuf::from(&pip.module)],
    };
    let lock: String = Sha256::digest(format!("{}\n{}", pip.python, pip.requirements).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let marker = folder.join(".installed");
    let _held = INSTALLING.lock().await;
    if std::fs::read_to_string(&marker).is_ok_and(|text| text == lock) && python.is_file() {
        return Ok(program);
    }
    let home = tools.join(&tool.name).join("home");
    hoista_system::paths::private_directory(&home).map_err(|_| EngineError::Store)?;
    let requirements = folder.join("requirements.txt");
    std::fs::write(&requirements, &pip.requirements).map_err(|_| EngineError::Store)?;
    if venv.exists() {
        std::fs::remove_dir_all(&venv).map_err(|_| EngineError::Store)?;
    }
    let venv_text = venv.to_string_lossy().into_owned();
    let python_text = python.to_string_lossy().into_owned();
    let requirements_text = requirements.to_string_lossy().into_owned();
    uv_run(
        &uv,
        tools,
        &home,
        &[
            "venv",
            "--quiet",
            "--no-config",
            "--managed-python",
            "--python",
            &pip.python,
            &venv_text,
        ],
    )
    .await?;
    uv_run(
        &uv,
        tools,
        &home,
        &[
            "pip",
            "install",
            "--quiet",
            "--no-config",
            "--require-hashes",
            "--only-binary",
            ":all:",
            "--default-index",
            "https://pypi.org/simple",
            "--python",
            &python_text,
            "-r",
            &requirements_text,
        ],
    )
    .await?;
    if !python.is_file() {
        return Err(missing(
            "Hoista couldn't set up Python for this provider's command-line tool.",
        ));
    }
    std::fs::write(&marker, &lock).map_err(|_| EngineError::Store)?;
    Ok(program)
}

/// Runs Hoista's uv with its own home, cache and Python folder.
async fn uv_run(uv: &Path, tools: &Path, home: &Path, args: &[&str]) -> Result<(), EngineError> {
    let mut command = tokio::process::Command::new(uv);
    command
        .args(args)
        .env_clear()
        .current_dir(home)
        .stdin(Stdio::null());
    hoista_system::process::inherit_environment(&mut command);
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("UV_CACHE_DIR", tools.join("uv-cache"))
        .env("UV_PYTHON_INSTALL_DIR", tools.join("python"))
        .env("UV_PYTHON_INSTALL_BIN", "0")
        .env("UV_PYTHON_INSTALL_REGISTRY", "0")
        .env("UV_NO_PROGRESS", "1")
        .env("NO_COLOR", "1");
    let (status, _, errors) = hoista_system::process::capture_all(
        &mut command,
        None,
        MAX_OUTPUT,
        INSTALL_TIMEOUT,
    )
    .await
    .map_err(|_| missing("Hoista couldn't install this provider's command-line tool. Check the connection and try again."))?;
    if status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&errors)
        .lines()
        .map(str::trim)
        .rev()
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_owned();
    Err(EngineError::Provider {
        evidence: None,
        code: "provider.tool_unavailable".into(),
        message: format!("Hoista couldn't install this provider's command-line tool: {said}"),
    })
}

/// The parts of a token that comes in parts, each by how it starts:
/// Modal's `ak-…` ID and `as-…` secret, from the line its dashboard shows.
fn token_parts(tool: &Tool, token: &Secret) -> Result<Vec<(String, Secret)>, EngineError> {
    let words: Vec<&str> = token
        .expose()
        .split(|c: char| c.is_whitespace() || c == ':' || c == '=' || c == ',')
        .map(|word| word.trim_matches(|c| c == '"' || c == '\''))
        .filter(|word| !word.is_empty())
        .collect();
    tool.token_parts
        .iter()
        .map(|(variable, start)| {
            words
                .iter()
                .find(|word| word.starts_with(start.as_str()) && plain(word))
                .map(|word| (variable.clone(), Secret::new((*word).to_owned())))
                .ok_or_else(|| EngineError::Provider {
                    evidence: None,
                    code: "provider.token_expired".into(),
                    message: format!("The saved token is missing its part that starts {start}. Reconnect the account with the whole token."),
                })
        })
        .collect()
}

/// The verified binary, downloading it the first time.
async fn installed(tool: &Tool, tools: &Path) -> Result<PathBuf, EngineError> {
    let platform = platform().ok_or(missing(
        "Hoista can't run this provider's command-line tool on this kind of computer.",
    ))?;
    let download = tool
        .downloads
        .iter()
        .find(|download| download.platform == platform)
        .ok_or(missing(
            "Hoista can't run this provider's command-line tool on this kind of computer.",
        ))?;
    let folder = tools.join(&tool.name).join(&tool.version);
    let binary = folder.join(Path::new(&download.inside).file_name().unwrap_or_default());
    let marker = folder.join(".verified");
    let _held = INSTALLING.lock().await;
    if std::fs::read_to_string(&marker).is_ok_and(|text| text == download.integrity)
        && binary.is_file()
    {
        // A binary from a zip may lack its mode; set it again.
        executable(&binary)?;
        return Ok(binary);
    }
    for level in [tools, &tools.join(&tool.name), &folder] {
        hoista_system::paths::private_directory(level).map_err(|_| EngineError::Store)?;
    }
    let archive = fetch(download).await?;
    let folder_for_unpack = folder.clone();
    let inside = download.inside.clone();
    let zipped = download.archive == "zip";
    let binary_for_unpack = binary.clone();
    tokio::task::spawn_blocking(move || {
        if zipped {
            unzip(&archive, &inside, &folder_for_unpack, &binary_for_unpack)
        } else {
            unpack(&archive, &inside, &folder_for_unpack, &binary_for_unpack)
        }
    })
    .await
    .map_err(|_| EngineError::Store)??;
    std::fs::write(&marker, &download.integrity).map_err(|_| EngineError::Store)?;
    Ok(binary)
}

/// Downloads one pinned archive and checks it before anything uses it.
async fn fetch(download: &Download) -> Result<Vec<u8>, EngineError> {
    use base64::Engine as _;
    use futures_util::StreamExt as _;
    use sha2::{Digest, Sha256, Sha512};
    let failed = || {
        missing(
            "Hoista couldn't download this provider's command-line tool. Check the connection and try again.",
        )
    };
    // The checksum, not the connection, decides whether the file is trusted,
    // so this follows the computer's proxy settings like the tool itself will.
    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
        .connect_timeout(DOWNLOAD_CONNECT_TIMEOUT)
        .timeout(DOWNLOAD_TIMEOUT)
        .user_agent(concat!("hoista/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| failed())?;
    let response = client
        .get(&download.url)
        .send()
        .await
        .map_err(|_| failed())?;
    if !response.status().is_success() {
        return Err(failed());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| failed())?;
        if bytes.len() as u64 + chunk.len() as u64 > MAX_DOWNLOAD {
            return Err(failed());
        }
        bytes.extend_from_slice(&chunk);
    }
    let actual = match download.integrity.split_once('-') {
        Some(("sha512", _)) => format!(
            "sha512-{}",
            base64::engine::general_purpose::STANDARD.encode(Sha512::digest(&bytes))
        ),
        Some(("sha256", _)) => format!(
            "sha256-{}",
            base64::engine::general_purpose::STANDARD.encode(Sha256::digest(&bytes))
        ),
        _ => return Err(EngineError::Store),
    };
    if actual != download.integrity {
        return Err(missing(
            "The downloaded command-line tool didn't match the copy Hoista expects, so Hoista didn't use it.",
        ));
    }
    Ok(bytes)
}

/// Takes the one binary out of a checked zip.
fn unzip(archive: &[u8], inside: &str, folder: &Path, binary: &Path) -> Result<(), EngineError> {
    let mut entries =
        zip::ZipArchive::new(std::io::Cursor::new(archive)).map_err(|_| EngineError::Store)?;
    let Ok(mut entry) = entries.by_name(inside) else {
        return Err(missing(
            "The downloaded command-line tool was missing its program, so Hoista didn't use it.",
        ));
    };
    if !entry.is_file() {
        return Err(EngineError::Store);
    }
    let partial = folder.join(".download");
    let mut file = std::fs::File::create(&partial).map_err(|_| EngineError::Store)?;
    std::io::copy(&mut entry, &mut file).map_err(|_| EngineError::Store)?;
    file.sync_all().map_err(|_| EngineError::Store)?;
    executable(&partial)?;
    std::fs::rename(&partial, binary).map_err(|_| EngineError::Store)
}

/// Lets the unpacked program run: a zip doesn't carry the mode a tarball
/// does, so a CLI shipped as a zip (Render's) can't run without this.
fn executable(path: &Path) -> Result<(), EngineError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| EngineError::Store)?;
    }

    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Takes the one binary out of a checked archive.
fn unpack(archive: &[u8], inside: &str, folder: &Path, binary: &Path) -> Result<(), EngineError> {
    let mut entries = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let partial = folder.join(".download");
    for entry in entries.entries().map_err(|_| EngineError::Store)? {
        let mut entry = entry.map_err(|_| EngineError::Store)?;
        let path = entry.path().map_err(|_| EngineError::Store)?;
        if path != Path::new(inside) || !entry.header().entry_type().is_file() {
            continue;
        }
        let mut file = std::fs::File::create(&partial).map_err(|_| EngineError::Store)?;
        std::io::copy(&mut entry, &mut file).map_err(|_| EngineError::Store)?;
        file.sync_all().map_err(|_| EngineError::Store)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| EngineError::Store)?;
        }
        std::fs::rename(&partial, binary).map_err(|_| EngineError::Store)?;
        return Ok(());
    }
    Err(missing(
        "The downloaded command-line tool was missing its program, so Hoista didn't use it.",
    ))
}

/// A path argument, kept inside the project's folder: relative, no `..`.
/// Storage addresses such as `ss:///bucket/file` are not paths.
fn inside_folder(value: &str) -> bool {
    if value.contains("://") {
        return true;
    }
    let path = Path::new(value);
    !value.is_empty()
        && path
            .components()
            .all(|resource| matches!(resource, Component::Normal(_) | Component::CurDir))
}

fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// The argument list for one command, checked against its catalog entry.
fn arguments(command: &Command, args: Value) -> Result<(Vec<String>, bool), EngineError> {
    let invalid = |message: String| EngineError::Provider {
        evidence: None,
        code: "provider.invalid_argument".into(),
        message,
    };
    let given = match args {
        Value::Null => Map::new(),
        Value::Object(map) => map,
        _ => return Err(EngineError::Invalid("Give arguments as an object.")),
    };
    let name = &command.name;
    if let Some(unknown) = given.keys().find(|key| {
        !command.args.iter().any(|arg| &arg.name == *key)
            && !command.flags.iter().any(|flag| &flag.name == *key)
    }) {
        return Err(invalid(format!("{name} has no argument {unknown}.")));
    }
    let mut uses_paths = false;
    let mut list: Vec<String> = command.words.clone();
    for arg in &command.args {
        let values: Vec<String> = match given.get(&arg.name) {
            None | Some(Value::Null) => {
                if arg.required {
                    return Err(invalid(format!("{name} needs {}.", arg.name)));
                }
                continue;
            }
            Some(Value::Array(items)) if arg.many => items.iter().filter_map(text).collect(),
            Some(value) => {
                vec![text(value).ok_or_else(|| invalid(format!("Give {} as text.", arg.name)))?]
            }
        };
        for value in values {
            // Something that reads as an option would change what runs.
            if value.starts_with('-') || value.is_empty() || value.contains('\0') {
                return Err(invalid(format!("{} can't start with a dash.", arg.name)));
            }
            if arg.path {
                if !inside_folder(&value) {
                    return Err(invalid(format!(
                        "{} must be a path inside the app's folder.",
                        arg.name
                    )));
                }
                uses_paths |= !value.contains("://");
            }
            list.push(value);
        }
    }
    for flag in &command.flags {
        let Some(value) = given.get(&flag.name) else {
            continue;
        };
        if flag.kind == "bool" {
            match value {
                Value::Bool(true) => list.push(format!("--{}", flag.name)),
                Value::Bool(false) | Value::Null => {}
                _ => return Err(invalid(format!("Give {} as true or false.", flag.name))),
            }
            continue;
        }
        let values: Vec<String> = match value {
            Value::Null => continue,
            Value::Array(items) => items.iter().filter_map(text).collect(),
            other => {
                vec![text(other).ok_or_else(|| invalid(format!("Give {} as text.", flag.name)))?]
            }
        };
        for value in values {
            if value.contains('\0') || value.contains('\n') {
                return Err(invalid(format!("{} can't span lines.", flag.name)));
            }
            if flag.kind == "integer" && value.parse::<i64>().is_err() {
                return Err(invalid(format!("Give {} as a whole number.", flag.name)));
            }
            if flag.kind == "choice" && !flag.choices.iter().any(|choice| choice == &value) {
                return Err(invalid(format!(
                    "{} is one of {}.",
                    flag.name,
                    flag.choices.join(", ")
                )));
            }
            if flag.path {
                if !inside_folder(&value) {
                    return Err(invalid(format!(
                        "{} must be a path inside the app's folder.",
                        flag.name
                    )));
                }
                uses_paths = true;
            }
            // One argument, `--name=value`, so a value is never read as an option.
            list.push(format!("--{}={value}", flag.name));
        }
    }
    Ok((list, uses_paths))
}

/// Where a command runs, all chosen by Hoista: what the resource it runs
/// on names (its project, service, environment, app, edge function…),
/// and the project's source folder.
pub(super) struct Aim<'a> {
    pub context: &'a [(&'a str, &'a str)],
    pub folder: Option<&'a Path>,
}

/// A name from the resource, safe to put in an option.
fn plain(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

/// Runs one CLI command where `aim` says and returns what it printed,
/// secrets removed.
///
/// # Errors
/// Unknown commands, invalid arguments, a missing folder, a tool Hoista
/// couldn't get, and the command's own failure, with its message.
pub(super) async fn run(
    tool: &Tool,
    http: &ProviderHttp,
    token: &Secret,
    aim: Aim<'_>,
    name: &str,
    args: Value,
) -> Result<Value, EngineError> {
    let Aim { context, folder } = aim;
    let command = tool
        .commands
        .iter()
        .find(|command| command.name == name)
        .ok_or(EngineError::Invalid(
            "This provider has no such command in Hoista. Call list_provider_operations for the list.",
        ))?;
    // On a server the token stays with the credential keeper.
    if token.handle_key().is_some() {
        return Err(EngineError::Provider {
            evidence: None,
            code: "provider.not_supported".into(),
            message: "Command-line operations run only where Hoista runs on your own computer for now. Use the provider's API operations here.".into(),
        });
    }
    let tools = http.tools().ok_or(EngineError::Store)?;
    // The variables and options that aim it, from what the resource names.
    let mut variables = Vec::new();
    for (variable, from) in &tool.target_variables {
        if let Some((_, value)) = context.iter().find(|(key, _)| key == from) {
            if !plain(value) {
                return Err(EngineError::Invalid(
                    "This resource's name can't be used to aim a command.",
                ));
            }
            variables.push((variable.clone(), (*value).to_owned()));
        }
    }
    let mut aimed = Vec::new();
    for flag in &command.targets {
        let Some(from) = tool.targets.get(flag) else {
            continue;
        };
        if let Some((_, value)) = context.iter().find(|(key, _)| key == from) {
            if !plain(value) {
                return Err(EngineError::Invalid(
                    "This resource's name can't be used to aim a command.",
                ));
            }
            aimed.push(format!("--{flag}={value}"));
        }
    }
    // Every command is aimed: by its options, or by all the variables.
    if aimed.is_empty()
        && (tool.target_variables.is_empty() || variables.len() < tool.target_variables.len())
    {
        return Err(EngineError::Invalid(
            "This command runs on one resource: give the resource_id of one Hoista watches.",
        ));
    }
    // What the resource names (an edge function, say) fills its argument.
    let mut args = match args {
        Value::Null => Value::Object(Map::new()),
        other => other,
    };
    if let Value::Object(given) = &mut args {
        for (arg, from) in &tool.context {
            if command.takes(arg)
                && let Some((_, value)) = context.iter().find(|(key, _)| key == from)
            {
                // What Hoista fills in from the resource always wins.
                if given
                    .get(arg)
                    .is_some_and(|said| said.as_str() != Some(*value))
                {
                    return Err(EngineError::Provider {
                        evidence: None,
                        code: "provider.argument_filled_in".into(),
                        message: format!("Hoista fills in {arg} from the resource; leave it out."),
                    });
                }
                given.insert(arg.clone(), Value::String((*value).to_owned()));
            }
        }
    }
    let (mut list, uses_paths) = arguments(command, args)?;
    let folder = match folder {
        Some(folder) if folder.is_dir() => Some(folder.to_path_buf()),
        _ if command.folder || uses_paths => {
            return Err(EngineError::Provider {
                evidence: None,
                code: "provider.needs_folder".into(),
                message: format!(
                    "{name} works on the app's source folder, and this app has none linked in Hoista. Link the app to its folder, then run it again."
                ),
            });
        }
        _ => None,
    };
    let program = program(tool, tools).await?;

    // Its own home: never the person's CLI sign-in, config or cache.
    let home = tools.join(&tool.name).join("home");
    hoista_system::paths::private_directory(&home).map_err(|_| EngineError::Store)?;
    let scratch = tempfile::Builder::new()
        .prefix("run-")
        .tempdir_in(&home)
        .map_err(|_| EngineError::Store)?;
    let workdir = folder.unwrap_or_else(|| scratch.path().to_path_buf());
    list.extend(aimed);
    list.extend(command.fixed.iter().cloned());
    list.extend(tool.always.iter().cloned());
    if command.json
        && let Some(flag) = &tool.json_flag
    {
        list.push(format!("--{flag}"));
    }
    if let Some(option) = &tool.workdir {
        list.push(format!("{option}{}", workdir.display()));
    }
    if command.risk != ProviderRisk::Read {
        list.extend(tool.confirmed.iter().cloned());
        if command.yes
            && let Some(flag) = &tool.yes_flag
        {
            list.push(format!("--{flag}"));
        }
    }

    let mut process = tokio::process::Command::new(&program.path);
    process
        .args(&program.prefix)
        .args(&list)
        .env_clear()
        .current_dir(&workdir)
        .stdin(Stdio::null());
    hoista_system::process::inherit_environment(&mut process);
    process
        .env("HOME", scratch.path())
        .env("USERPROFILE", scratch.path())
        .env("XDG_CONFIG_HOME", scratch.path().join(".config"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("NO_COLOR", "1")
        .env("CI", "1");
    for (key, value) in &tool.variables {
        process.env(key, value);
    }
    for (key, value) in &variables {
        process.env(key, value);
    }
    let mut secrets = vec![Secret::new(token.expose().to_owned())];
    if tool.token_parts.is_empty() {
        process.env(&tool.token_variable, token.expose());
    } else {
        for (variable, part) in token_parts(tool, token)? {
            process.env(variable, part.expose());
            secrets.push(part);
        }
    }
    // A change runs to the end even when the caller stops waiting (an agent's
    // call timing out): stopping `modal deploy` or `railway up` halfway
    // leaves the provider with half a deploy. Reads stop with their caller.
    let finished = if command.risk == ProviderRisk::Read {
        hoista_system::process::capture_all(&mut process, None, MAX_OUTPUT, COMMAND_TIMEOUT).await
    } else {
        tokio::spawn(async move {
            let done = hoista_system::process::capture_all(
                &mut process,
                None,
                MAX_OUTPUT,
                COMMAND_TIMEOUT,
            )
            .await;
            drop(scratch);
            done
        })
        .await
        .unwrap_or(Err(hoista_system::process::ProcessError::Cleanup))
    };
    let (status, out, errors) = finished.map_err(|error| match error {
        hoista_system::process::ProcessError::Timeout => EngineError::Provider {
            evidence: None,
            code: "provider.timeout".into(),
            message: format!("{name} ran out of time and was stopped. Check the provider before running it again."),
        },
        _ => missing("Hoista couldn't run this provider's command-line tool."),
    })?;

    let scanner = LeakScanner::new().map_err(|_| EngineError::Store)?;
    let printed = scanner.redact(&plain_text(&String::from_utf8_lossy(&out)), &secrets);
    let messages = scanner.redact(&plain_text(&String::from_utf8_lossy(&errors)), &secrets);
    let mut output = parse(&printed);
    super::api_catalog::scrub(&mut output, &scanner, token);
    let messages = tail(&messages, MAX_MESSAGE_CHARS);
    let shown = std::iter::once(tool.name.clone())
        .chain(list.iter().cloned())
        .filter(|arg| {
            tool.workdir
                .as_ref()
                .is_none_or(|option| !arg.starts_with(option))
        })
        .collect::<Vec<_>>()
        .join(" ");
    if !status.success() {
        let said = output["error"]["message"]
            .as_str()
            .or_else(|| output["error"].as_str())
            .or_else(|| output["message"].as_str())
            .map(str::to_owned)
            .or_else(|| reason(&messages))
            .or_else(|| reason(&printed))
            .unwrap_or_else(|| "it gave no reason".into());
        return Err(EngineError::Provider {
            evidence: None,
            code: "provider.command_failed".into(),
            message: format!("{name} failed: {said}"),
        });
    }
    Ok(json!({
        "command": shown,
        "output": output,
        "messages": messages,
    }))
}

/// Text without terminal colours and styles.
fn plain_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                // Parameters, then one final letter.
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() || next == '~' {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// The line that says why a command failed: its error line if it has one,
/// otherwise its last line that isn't housekeeping.
fn reason(text: &str) -> Option<String> {
    // A boxed error, as Rich-based CLIs (Modal's) print it.
    if let Some(start) = text
        .lines()
        .position(|line| line.trim_start().starts_with('╭') && line.contains("Error"))
    {
        let inside: Vec<&str> = text
            .lines()
            .skip(start + 1)
            .take_while(|line| !line.trim_start().starts_with('╰'))
            .map(|line| line.trim().trim_matches('│').trim())
            .filter(|line| !line.is_empty())
            .collect();
        if !inside.is_empty() {
            return Some(inside.join(" "));
        }
    }
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let error = lines.iter().rposition(|line| {
        let lower = line.to_ascii_lowercase();
        line.starts_with('✘') || lower.contains("[error]") || lower.starts_with("error")
    });
    // Wrangler says which request failed on its error line and why on the
    // lines after it ("Authentication error [code: 10000]"): keep both.
    if let Some(at) = error {
        let headline = lines[at].trim_start_matches(['✘', '▲', ' ']);
        let detail: Vec<&str> = lines[at + 1..]
            .iter()
            .copied()
            .take_while(|line| {
                let lower = line.to_ascii_lowercase();
                !line.starts_with('✘')
                    && !line.starts_with('▲')
                    && !lower.contains("logs were written")
                    && !lower.contains("create an issue")
                    && !lower.starts_with("if you think this is a bug")
            })
            .take(MAX_DETAIL_LINES)
            .collect();
        let said = if detail.is_empty() {
            headline.to_owned()
        } else {
            format!("{headline} {}", detail.join(" "))
        };
        return Some(match cloudflare_hint(&said) {
            Some(hint) => format!("{said} {hint}"),
            None => said,
        });
    }
    // No error line: the last line that isn't housekeeping.
    lines
        .iter()
        .rev()
        .find(|line| {
            let lower = line.to_ascii_lowercase();
            !lower.contains("logs were written")
                && !lower.contains("newer version")
                && !lower.contains("create an issue")
        })
        .map(|line| line.trim_start_matches(['✘', '▲', ' ']).to_owned())
}

/// What Cloudflare's own error codes mean for the token, in its words.
fn cloudflare_hint(said: &str) -> Option<&'static str> {
    if said.contains("[code: 10000]") || said.contains("Authentication error") {
        Some(
            "Cloudflare refused the token for this: it needs Account → Workers Scripts → Edit, with this account under Account Resources. Edit the token in Cloudflare (My Profile → API Tokens), then try again.",
        )
    } else if said.contains("workers.dev subdomain") {
        Some(
            "This Cloudflare account has no workers.dev subdomain yet: open Workers & Pages in the Cloudflare dashboard once to choose one, then try again.",
        )
    } else if said.contains("[code: 7003]") || said.contains("[code: 10001]") {
        Some(
            "The token doesn't reach this account: add it under Account Resources on the token in Cloudflare.",
        )
    } else {
        None
    }
}

/// JSON, one JSON value per line, or plain text.
fn parse(printed: &str) -> Value {
    let trimmed = printed.trim();
    if trimmed.is_empty() {
        return Value::Null;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return value;
    }
    let lines: Option<Vec<Value>> = trimmed
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).ok())
        .collect();
    lines.map_or_else(|| Value::String(trimmed.to_owned()), Value::Array)
}

fn tail(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.len() <= max {
        return text.to_owned();
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &text[start..])
}

#[cfg(test)]
mod tests {
    #[test]
    fn keeps_cloudflares_reason_after_wranglers_error_line() {
        let printed = "⛅️ wrangler 4.0.0\n✘ [ERROR] A request to the Cloudflare API (/accounts/abc/workers/scripts/edge) failed.\n\n  Authentication error [code: 10000]\n\nIf you think this is a bug then please create an issue\n";
        let said = super::reason(printed).expect("a reason");
        assert!(
            said.starts_with("[ERROR] A request to the Cloudflare API"),
            "{said}"
        );
        assert!(
            said.contains("Authentication error [code: 10000]"),
            "{said}"
        );
        assert!(said.contains("Workers Scripts → Edit"), "{said}");
    }

    use super::*;

    fn tool() -> &'static Tool {
        super::super::supabase::manifest().cli.as_ref().unwrap()
    }

    #[test]
    fn the_catalog_matches_the_pinned_cli_and_leaves_out_what_it_should() {
        let ops = operations(tool());
        assert!(ops.len() > 60, "{} commands", ops.len());
        let find = |name: &str| ops.iter().find(|op| op.name == name);
        for gone in [
            "supabase login",
            "supabase link",
            "supabase secrets set",
            "supabase projects api-keys",
            "supabase encryption get-root-key",
            "supabase start",
            "supabase projects delete",
        ] {
            assert!(find(gone).is_none(), "{gone} must not be offered");
        }
        for op in &ops {
            for arg in &op.args {
                assert!(
                    ![
                        "remote",
                        "password",
                        "db-url",
                        "db-password",
                        "project-ref",
                        "token"
                    ]
                    .contains(&arg.name.as_str()),
                    "{} lets callers set {}",
                    op.name,
                    arg.name
                );
            }
        }
        assert_eq!(
            find("supabase db push").map(|op| op.risk),
            Some(ProviderRisk::Destroy)
        );
        assert_eq!(
            find("supabase inspect db locks").map(|op| op.risk),
            Some(ProviderRisk::Read)
        );
        assert_eq!(
            find("supabase functions deploy").map(|op| op.risk),
            Some(ProviderRisk::Change)
        );
        assert_eq!(
            find("supabase gen types").map(|op| op.risk),
            Some(ProviderRisk::Read)
        );
        assert_eq!(
            find("supabase storage rm").map(|op| op.risk),
            Some(ProviderRisk::Destroy)
        );
        // An edge function's own commands know which function they're on.
        let deploy = find("supabase functions deploy").unwrap();
        let arg = deploy
            .args
            .iter()
            .find(|arg| arg.name == "function_name")
            .unwrap();
        assert_eq!(arg.filled_in.as_deref(), Some("function"));
        assert!(!arg.required);
        // Every platform has a pinned, checksummed build.
        for platform in ["darwin-arm64", "darwin-x64", "linux-x64", "windows-x64"] {
            let download = tool().downloads.iter().find(|d| d.platform == platform);
            assert!(download.is_some_and(|d| d.integrity.starts_with("sha512-")
                && d.url.starts_with("https://registry.npmjs.org/")));
        }
    }

    #[test]
    fn arguments_are_checked_and_never_read_as_options() {
        let command = tool()
            .commands
            .iter()
            .find(|c| c.name == "supabase db push")
            .unwrap();
        let (list, paths) =
            arguments(command, json!({"dry-run": true, "include-all": false})).unwrap();
        assert_eq!(list, ["db", "push", "--dry-run"]);
        assert!(!paths);
        assert!(arguments(command, json!({"password": "x"})).is_err());
        assert!(arguments(command, json!({"dry-run": "yes"})).is_err());

        let deploy = tool()
            .commands
            .iter()
            .find(|c| c.name == "supabase functions deploy")
            .unwrap();
        let (list, _) = arguments(
            deploy,
            json!({"function_name": ["hello", "bye"], "jobs": 2}),
        )
        .unwrap();
        assert_eq!(list, ["functions", "deploy", "hello", "bye", "--jobs=2"]);
        assert!(arguments(deploy, json!({"function_name": "--prune"})).is_err());
        assert!(arguments(deploy, json!({"jobs": "many"})).is_err());
        assert!(arguments(deploy, json!({"import-map": "../../etc/passwd"})).is_err());
        assert!(arguments(deploy, json!({"import-map": "/etc/passwd"})).is_err());
        let (list, paths) = arguments(
            deploy,
            json!({"import-map": "supabase/functions/import_map.json"}),
        )
        .unwrap();
        assert!(paths);
        assert_eq!(
            list.last().unwrap(),
            "--import-map=supabase/functions/import_map.json"
        );

        let copy = tool()
            .commands
            .iter()
            .find(|c| c.name == "supabase storage cp")
            .unwrap();
        let (_, paths) = arguments(
            copy,
            json!({"src": "ss:///avatars/a.png", "dst": "ss:///exports/a.png"}),
        )
        .unwrap();
        assert!(!paths, "storage addresses aren't files on this computer");
        assert!(
            arguments(
                copy,
                json!({"src": "/Users/sam/.ssh/id_ed25519", "dst": "ss:///exports/k"})
            )
            .is_err()
        );

        let types = tool()
            .commands
            .iter()
            .find(|c| c.name == "supabase gen types")
            .unwrap();
        assert!(arguments(types, json!({"lang": "cobol"})).is_err());
        assert!(arguments(types, json!({"lang": "typescript"})).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_program_from_a_zip_can_run() {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt as _;
        let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        archive
            .start_file("cli", zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(b"#!/bin/sh\n").unwrap();
        let bytes = archive.finish().unwrap().into_inner();
        let folder = tempfile::tempdir().unwrap();
        let binary = folder.path().join("cli");
        unzip(&bytes, "cli", folder.path(), &binary).unwrap();
        let mode = std::fs::metadata(&binary).unwrap().permissions().mode();
        assert_eq!(mode & 0o700, 0o700);
    }

    #[test]
    fn a_boxed_error_reads_as_its_text() {
        let printed = "╭─ Error ─────────╮\n│ Token missing. Could │\n│ not authenticate.    │\n╰─────────────────╯\n";
        assert_eq!(
            reason(printed).as_deref(),
            Some("Token missing. Could not authenticate.")
        );
    }

    #[test]
    fn output_is_json_lines_or_text() {
        assert_eq!(parse("{\"a\":1}"), json!({"a":1}));
        assert_eq!(parse("{\"a\":1}\n{\"b\":2}\n"), json!([{"a":1},{"b":2}]));
        assert_eq!(parse("Finished.\n"), json!("Finished."));
        assert_eq!(parse("  "), Value::Null);
        assert_eq!(tail("abcdef", 3), "…def");
        assert_eq!(
            plain_text("\u{1b}[31m✘ \u{1b}[41;31m[ERROR]\u{1b}[0m fetch failed"),
            "✘ [ERROR] fetch failed"
        );
        assert_eq!(
            reason("▲ [WARNING] proxy\n✘ [ERROR] fetch failed\nLogs were written to x\n")
                .as_deref(),
            Some("[ERROR] fetch failed")
        );
        assert_eq!(
            reason("Deploying\nProject not found\n").as_deref(),
            Some("Project not found")
        );
    }

    #[test]
    fn only_the_pinned_program_is_unpacked() {
        use flate2::{Compression, write::GzEncoder};
        let mut archive = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
        for (path, body) in [
            ("package/README.md", &b"hello"[..]),
            ("package/bin/supabase", b"#!bin"),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive.append_data(&mut header, path, body).unwrap();
        }
        let bytes = archive.into_inner().unwrap().finish().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("supabase");
        unpack(&bytes, "package/bin/supabase", dir.path(), &binary).unwrap();
        assert_eq!(std::fs::read(&binary).unwrap(), b"#!bin");
        assert!(!dir.path().join("README.md").exists());
        assert!(
            unpack(
                &bytes,
                "package/bin/other",
                dir.path(),
                &dir.path().join("other")
            )
            .is_err()
        );
    }

    /// A run end to end, with a stand-in program where Hoista keeps the
    /// pinned one: what it's started with, and what comes back.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_run_is_aimed_isolated_and_redacted() {
        use std::os::unix::fs::PermissionsExt as _;
        let Some(platform) = platform() else { return };
        let tool = super::super::manifest::builtin("railway")
            .cli
            .as_ref()
            .unwrap();
        let download = tool
            .downloads
            .iter()
            .find(|d| d.platform == platform)
            .unwrap();
        let tools = tempfile::tempdir().unwrap();
        let folder = tools.path().join("railway").join(&tool.version);
        std::fs::create_dir_all(&folder).unwrap();
        let program = folder.join("railway");
        // Prints its arguments, where its home is, and the token it was given.
        std::fs::write(
            &program,
            "#!/bin/sh\nprintf '{\"args\":\"%s\",\"home\":\"%s\",\"said\":\"%s\",\"mine\":\"%s\"}' \"$*\" \"$HOME\" \"$RAILWAY_API_TOKEN\" \"${CARGO_PKG_NAME:-none}\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(folder.join(".verified"), &download.integrity).unwrap();
        let http = ProviderHttp::local_fixture("http://127.0.0.1:9".parse().unwrap())
            .with_tools(tools.path().to_path_buf());
        let token = Secret::new("rw_token_value_1234567890".into());
        let answer = run(
            tool,
            &http,
            &token,
            Aim {
                context: &[("project", "p1"), ("service", "s1"), ("environment", "e1")],
                folder: None,
            },
            "railway restart",
            Value::Null,
        )
        .await
        .expect("run");
        let output = &answer["output"];
        // Nothing of the daemon's own environment reaches the tool.
        assert_eq!(output["mine"], "none");
        let args = output["args"].as_str().unwrap();
        assert_eq!(
            args,
            "restart --project=p1 --service=s1 --environment=e1 --json --yes"
        );
        assert!(
            !output["said"].as_str().unwrap().contains("rw_token_value"),
            "token echoed back"
        );
        let home = output["home"].as_str().unwrap();
        assert!(home.starts_with(tools.path().join("railway").join("home").to_str().unwrap()));
        // A read gets no "yes"; a resource without a project can't be aimed.
        let read = run(
            tool,
            &http,
            &token,
            Aim {
                context: &[("project", "p1"), ("service", "s1")],
                folder: None,
            },
            "railway domain list",
            Value::Null,
        )
        .await
        .expect("read");
        assert_eq!(
            read["output"]["args"],
            "domain list --project=p1 --service=s1 --json"
        );
        assert!(
            run(
                tool,
                &http,
                &token,
                Aim {
                    context: &[("project", "-p")],
                    folder: None
                },
                "railway restart",
                Value::Null
            )
            .await
            .is_err()
        );
        assert!(
            run(
                tool,
                &http,
                &token,
                Aim {
                    context: &[],
                    folder: None
                },
                "railway restart",
                Value::Null
            )
            .await
            .is_err()
        );
    }

    /// Installs Wrangler from its pinned lockfile with this computer's Node
    /// and runs a command with a stand-in token: the answer is Cloudflare's
    /// refusal, not an install failure. Needs Node 22 and the npm registry.
    #[tokio::test]
    #[ignore = "downloads Wrangler from npm"]
    async fn a_node_cli_installs_from_its_lockfile_and_runs() {
        let tool = super::super::manifest::builtin("wrangler")
            .cli
            .as_ref()
            .unwrap();
        let tools = tempfile::tempdir().unwrap();
        let http = ProviderHttp::local_fixture("http://127.0.0.1:9".parse().unwrap())
            .with_tools(tools.path().to_path_buf());
        let token = Secret::new("cf_stand_in_token_value".into());
        let answer = run(
            tool,
            &http,
            &token,
            Aim {
                context: &[("account", "0123456789abcdef0123456789abcdef")],
                folder: None,
            },
            "wrangler d1 list",
            Value::Null,
        )
        .await;
        let said = format!("{answer:?}");
        assert!(!said.contains("couldn't install"), "{said}");
        assert!(!said.contains("cf_stand_in_token_value"), "{said}");
        assert!(
            tools
                .path()
                .join("wrangler/4.151.3/node_modules/wrangler/bin/wrangler.js")
                .is_file()
        );
        eprintln!("{said}");
    }

    /// Installs Modal's CLI with Hoista's pinned uv, its own Python and the
    /// hashed requirements, then runs a command with a stand-in token pair:
    /// the answer is Modal's refusal, not an install failure. Needs GitHub
    /// (uv and Python) and PyPI.
    #[tokio::test]
    #[ignore = "downloads uv, Python and Modal's packages"]
    async fn a_python_cli_installs_from_its_hashes_and_runs() {
        let tool = super::super::manifest::builtin("modal")
            .cli
            .as_ref()
            .unwrap();
        let tools = tempfile::tempdir().unwrap();
        let http = ProviderHttp::local_fixture("http://127.0.0.1:9".parse().unwrap())
            .with_tools(tools.path().to_path_buf());
        // Behind a proxy Hoista's own downloads can't pass, a checked uv can
        // be put in place first.
        if let Ok(uv) = std::env::var("HOISTA_TEST_UV") {
            let folder = tools.path().join("modal/1.6.3");
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::copy(uv, folder.join("uv")).unwrap();
            let platform = platform().unwrap();
            let download = tool
                .downloads
                .iter()
                .find(|d| d.platform == platform)
                .unwrap();
            std::fs::write(folder.join(".verified"), &download.integrity).unwrap();
        }
        let token =
            Secret::new("modal token set --token-id ak-standinstandinstand --token-secret as-standinsecretvalue".into());
        let mut said = String::new();
        for _ in 0..30 {
            let answer = run(
                tool,
                &http,
                &token,
                Aim {
                    context: &[("environment", "main")],
                    folder: None,
                },
                "modal app list",
                Value::Null,
            )
            .await;
            said = format!("{answer:?}");
            if !said.contains("provider.tool_installing") {
                break;
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
        assert!(!said.contains("couldn't install"), "{said}");
        assert!(!said.contains("tool_installing"), "{said}");
        assert!(!said.contains("as-standinsecretvalue"), "{said}");
        eprintln!("{said}");
        assert!(tools.path().join("modal/1.6.3/.installed").is_file());
    }
}
