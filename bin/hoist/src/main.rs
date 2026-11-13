//! `hoist`: Hoista's command line. Everything the desktop app does, from a
//! terminal or a script, on this computer's Hoista or on a remote one.

mod accounts;
mod agents;
mod bootstrap;
mod context;
mod deploys;
mod environment;
mod logs;
mod loopback;
mod observe;
mod plans;
mod projects;
mod remote;
mod resources;
mod service;
mod transfer;
mod ui;
mod upkeep;

use clap::{Args, CommandFactory, Parser, Subcommand};
use std::{path::PathBuf, process::ExitCode};

/// The top-level help, grouped the way people look for a command.
/// The exit status shells give a command stopped with Ctrl-C (128 + SIGINT).
const INTERRUPTED: u8 = 130;

const HELP: &str = "\
{about-with-newline}
{usage-heading} {usage}

Projects
  projects      List projects in the current environment
  status        Show a project: its resources, status and last deploy
  import        Bring in projects you already run on your providers
  link          Link a project to a folder or a GitHub repository
  map           Show how a project's resources connect
  resources     Restart, pause, move or delete a project's resources
  db            Look at and export a database

Deploying
  deploy        Deploy a folder
  plan          See what a deploy would do, and change it first
  deploys       List and inspect deploys
  rollback      Put an earlier deploy back
  cancel        Stop a deploy that's running
  env           A project's environment variables
  domains       A project's custom domains

Watching
  logs          A project's logs, recent or live
  metrics       CPU, memory and request charts the provider reports
  activity      What happened recently, by you and by agents
  billing       What each provider has billed this period
  usage         What the provider measured each day

Providers, environments and remotes
  accounts      Connected provider accounts: add, check, rotate a token, remove
  providers     The providers Hoista works with
  environments  Hoista environments, such as production and staging
  remote        Other Hoistas you use: on a server, in Docker or at a provider

Agents
  agents        Coding agents connected to Hoista, and what they may do
  mcp           Connect coding tools and web agents over MCP
  skills        Install Hoista's skills into Claude Code and Codex

Safety and upkeep
  backups       Backups, and moving Hoista to another computer
  operations    Provider operations Hoista started
  webhooks      Webhooks Hoista calls when things happen
  settings      Hoista's settings
  profile       Your name and initials

This computer
  start         Start Hoista's engine
  doctor        Check Hoista is installed and working
  diagnostics   A report for troubleshooting, without secrets
  daemon        The engine running on this computer
  service       Start Hoista at login
  tools         Where provider tools such as flyctl and wrangler are
  completion    Print a shell completion script

Options:
{options}

Run hoist COMMAND --help for a command's options.
Docs: https://docs.hoista.dev/cli/overview
";

/// Hoista's command line: your projects across your own hosting accounts.
///
/// Run a command without its arguments in a terminal and hoist asks for
/// them. In scripts, pass every argument; add --json for output to parse.
#[derive(Parser)]
#[command(
    name = "hoist",
    version,
    disable_help_subcommand = true,
    max_term_width = 100,
    help_template = HELP
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    global: Global,
}

/// Flags every command takes.
#[derive(Args, Clone)]
#[command(next_help_heading = "Global options")]
pub(crate) struct Global {
    /// Print JSON instead of tables and messages.
    #[arg(long, global = true)]
    pub(crate) json: bool,
    /// Answer yes to every confirmation.
    #[arg(short = 'y', long, global = true)]
    pub(crate) yes: bool,
    /// Never ask questions; fail with the flag to pass instead. Also on when CI is set.
    #[arg(long, global = true)]
    pub(crate) no_input: bool,
    /// Use this remote for one command instead of the one in use.
    #[arg(long, global = true, env = "HOISTA_REMOTE", value_name = "NAME")]
    pub(crate) remote: Option<String>,
    /// Use this environment for one command instead of the one in use.
    #[arg(
        short = 'e',
        long,
        global = true,
        env = "HOISTA_ENVIRONMENT",
        value_name = "NAME"
    )]
    pub(crate) environment: Option<String>,
    /// Where this computer's Hoista keeps its state. Default: ~/.hoista.
    #[arg(long, global = true, value_name = "PATH")]
    pub(crate) state_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    // Projects
    /// List projects in the current environment.
    #[command(display_order = 10)]
    Projects(projects::ProjectsArgs),
    /// Show a project: its resources, status and last deploy.
    #[command(display_order = 11)]
    Status(projects::StatusArgs),
    /// Bring projects you already run on your providers into Hoista.
    #[command(display_order = 12)]
    Import(projects::ImportArgs),
    /// Link a project to a folder or a GitHub repository.
    #[command(display_order = 13)]
    Link(projects::LinkArgs),
    /// Show how a project's resources connect.
    #[command(display_order = 14)]
    Map(projects::MapArgs),
    /// Work with a project's resources: services, databases, buckets.
    #[command(display_order = 15)]
    Resources(resources::ResourcesArgs),
    /// Look at and export a database.
    #[command(display_order = 16)]
    Db(resources::DbArgs),

    // Deploying
    /// Deploy a folder.
    #[command(display_order = 20)]
    Deploy(deploys::DeployArgs),
    /// See what a deploy would do, and change it before deploying.
    #[command(display_order = 21)]
    Plan(deploys::PlanArgs),
    /// List and inspect deploys.
    #[command(display_order = 22)]
    Deploys(deploys::DeploysArgs),
    /// Put an earlier deploy back.
    #[command(display_order = 23)]
    Rollback(deploys::RollbackArgs),
    /// Stop a deploy that's running.
    #[command(display_order = 24)]
    Cancel(deploys::CancelArgs),
    /// A project's environment variables.
    #[command(display_order = 25)]
    Env(deploys::EnvArgs),
    /// A project's custom domains.
    #[command(display_order = 26)]
    Domains(resources::DomainsArgs),

    // Watching
    /// A project's logs, recent or live.
    #[command(display_order = 30)]
    Logs(logs::LogsArgs),
    /// CPU, memory and request charts the provider reports.
    #[command(display_order = 31)]
    Metrics(observe::MetricsArgs),
    /// What happened recently, by you and by agents.
    #[command(display_order = 32)]
    Activity(observe::ActivityArgs),
    /// What each provider has billed this period, and plan limits.
    #[command(display_order = 33)]
    Billing(observe::BillingArgs),
    /// What the provider measured each day for a project.
    #[command(display_order = 34)]
    Usage(observe::UsageArgs),

    // Providers, environments and remotes
    /// Connected provider accounts: add, check, rotate a token, remove.
    #[command(display_order = 40)]
    Accounts(accounts::AccountsArgs),
    /// The providers Hoista works with.
    #[command(display_order = 41)]
    Providers,
    /// Hoista environments, such as production and staging.
    #[command(display_order = 42)]
    Environments(projects::EnvironmentsArgs),
    /// Other Hoistas you use: on a server, in Docker or at a provider.
    #[command(display_order = 43)]
    Remote(remote::RemoteArgs),

    // Agents
    /// Coding agents connected to Hoista, and what they may do.
    #[command(display_order = 50)]
    Agents(agents::AgentsArgs),
    /// Connect coding tools and web agents over MCP.
    #[command(display_order = 51)]
    Mcp(agents::McpArgs),
    /// Install Hoista's skills into Claude Code and Codex.
    #[command(display_order = 52)]
    Skills(agents::SkillsArgs),

    // Safety and upkeep
    /// Backups of Hoista's state, and moving it to another computer.
    #[command(display_order = 60)]
    Backups(upkeep::BackupsArgs),
    /// Provider operations Hoista started, and checking unclear ones.
    #[command(display_order = 61)]
    Operations(upkeep::OperationsArgs),
    /// Webhooks Hoista calls when things happen.
    #[command(display_order = 62)]
    Webhooks(upkeep::WebhooksArgs),
    /// Hoista's settings.
    #[command(display_order = 63)]
    Settings(upkeep::SettingsArgs),
    /// Your name and initials, shown on what you do.
    #[command(display_order = 64)]
    Profile(upkeep::ProfileArgs),

    // This computer
    /// Start Hoista's engine on this computer.
    #[command(display_order = 70)]
    Start,
    /// Check Hoista is installed and working.
    #[command(display_order = 71)]
    Doctor(upkeep::DoctorArgs),
    /// A report for troubleshooting, without secrets.
    #[command(display_order = 72)]
    Diagnostics(upkeep::DiagnosticsArgs),
    /// The engine running on this computer.
    #[command(display_order = 73)]
    Daemon(upkeep::DaemonArgs),
    /// Start Hoista at login.
    #[command(display_order = 74)]
    Service(upkeep::ServiceArgs),
    /// Where provider tools such as flyctl and wrangler are.
    #[command(display_order = 75)]
    Tools(upkeep::ToolsArgs),
    /// Print a shell completion script.
    #[command(display_order = 76)]
    Completion(CompletionArgs),
    /// Print the JSON Schema of Hoista's API types.
    #[command(display_order = 77, hide = true)]
    Schema(upkeep::SchemaArgs),
}

#[derive(Args)]
#[command(
    after_help = "Load it in zsh: hoist completion zsh > \"${fpath[1]}/_hoist\"\nIn bash: hoist completion bash > ~/.local/share/bash-completion/completions/hoist\nIn fish: hoist completion fish > ~/.config/fish/completions/hoist.fish"
)]
struct CompletionArgs {
    /// bash, zsh, fish, powershell or elvish.
    shell: clap_complete::Shell,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let _ = error.print();
            return ExitCode::from(if error.use_stderr() { 2 } else { 0 });
        }
    };
    ui::init(cli.global.json, cli.global.yes, cli.global.no_input);
    match run(cli).await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            if let Some(dialoguer::Error::IO(io)) = error.downcast_ref::<dialoguer::Error>()
                && io.kind() == std::io::ErrorKind::Interrupted
            {
                return ExitCode::from(INTERRUPTED);
            }
            eprintln!("{} {error:#}", console::style("Error:").red().bold());
            ExitCode::from(if error.downcast_ref::<ui::Usage>().is_some() {
                2
            } else {
                1
            })
        }
    }
}

async fn run(cli: Cli) -> anyhow::Result<bool> {
    let global = cli.global;
    let context = context::Context::new(
        global.state_dir.clone(),
        global.remote.clone(),
        global.environment.clone(),
    )?;
    let Some(command) = cli.command else {
        Cli::command().print_help()?;
        return Ok(true);
    };
    match command {
        Command::Projects(args) => projects::projects(&context, args).await?,
        Command::Status(args) => projects::status(&context, args).await?,
        Command::Import(args) => projects::import(&context, args).await?,
        Command::Link(args) => projects::link(&context, args).await?,
        Command::Map(args) => projects::map(&context, args).await?,
        Command::Environments(args) => projects::environments(&context, args).await?,
        Command::Resources(args) => resources::resources(&context, args).await?,
        Command::Db(args) => resources::db(&context, args).await?,
        Command::Domains(args) => resources::domains(&context, args).await?,
        Command::Deploy(args) => return deploys::deploy(&context, args).await,
        Command::Plan(args) => deploys::plan(&context, args).await?,
        Command::Deploys(args) => deploys::deploys(&context, args).await?,
        Command::Rollback(args) => deploys::rollback(&context, args).await?,
        Command::Cancel(args) => deploys::cancel(&context, args).await?,
        Command::Env(args) => deploys::env(&context, args).await?,
        Command::Logs(args) => logs::run(&context, args).await?,
        Command::Metrics(args) => observe::metrics(&context, args).await?,
        Command::Activity(args) => observe::activity(&context, args).await?,
        Command::Billing(args) => observe::billing(&context, args).await?,
        Command::Usage(args) => observe::usage(&context, args).await?,
        Command::Accounts(args) => accounts::accounts(&context, args).await?,
        Command::Providers => accounts::providers(&context).await?,
        Command::Remote(args) => return remote::remote(&context, args).await,
        Command::Agents(args) => agents::agents(&context, args).await?,
        Command::Mcp(args) => agents::mcp(&context, args).await?,
        Command::Skills(args) => agents::skills(args)?,
        Command::Backups(args) => upkeep::backups(&context, args).await?,
        Command::Operations(args) => upkeep::operations(&context, args).await?,
        Command::Webhooks(args) => upkeep::webhooks(&context, args).await?,
        Command::Settings(args) => upkeep::settings(&context, args).await?,
        Command::Profile(args) => upkeep::profile(&context, args).await?,
        Command::Start => upkeep::start(&context).await?,
        Command::Doctor(args) => return upkeep::doctor(&context, args).await,
        Command::Diagnostics(args) => upkeep::diagnostics(&context, args).await?,
        Command::Daemon(args) => upkeep::daemon(&context, args).await?,
        Command::Service(args) => upkeep::service(&context, args).await?,
        Command::Tools(args) => upkeep::tools(&context, args)?,
        Command::Completion(args) => {
            use std::io::Write as _;
            let mut script = Vec::new();
            clap_complete::generate(args.shell, &mut Cli::command(), "hoist", &mut script);
            // A closed pipe (`| head`) isn't a failure.
            let _ = std::io::stdout().write_all(&script);
        }
        Command::Schema(args) => upkeep::schema(&args)?,
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_tree_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn the_grouped_help_lists_every_command() {
        let listed: Vec<String> = HELP
            .lines()
            .filter_map(|line| {
                line.strip_prefix("  ")?
                    .split_whitespace()
                    .next()
                    .map(str::to_owned)
            })
            .collect();
        let commands: Vec<String> = Cli::command()
            .get_subcommands()
            .filter(|command| !command.is_hide_set())
            .map(|command| command.get_name().to_owned())
            .collect();
        assert_eq!(listed, commands);
    }

    #[test]
    fn old_command_names_are_gone() {
        for old in [
            "found", "connect", "hosts", "add", "mode", "source", "discover", "requests", "apps",
            "export", "database",
        ] {
            assert!(
                Cli::try_parse_from(["hoist", old]).is_err(),
                "{old} should not parse"
            );
        }
    }

    #[test]
    fn global_flags_go_anywhere() {
        let cli = Cli::try_parse_from(["hoist", "projects", "--json", "-e", "production"]).unwrap();
        assert!(cli.global.json);
        assert_eq!(cli.global.environment.as_deref(), Some("production"));
        let cli = Cli::try_parse_from(["hoist", "--remote", "office", "status", "api"]).unwrap();
        assert_eq!(cli.global.remote.as_deref(), Some("office"));
    }
}
