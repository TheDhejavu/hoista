//! Fills an empty state folder with the demo world: Sam's profile, accounts,
//! projects and their history, written through Hoista's own store so the
//! records are exactly what the engine keeps.
//!
//! Writing a record also writes an activity entry stamped now, so the records
//! are written into a scratch database and copied over (see `main.rs`) beside
//! [`timeline`], and the Activity screen reads as weeks of use rather than one
//! busy second.

use crate::{build::code_folder, world};
use anyhow::Context;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use hoista_server::application::ports::catalog::CatalogRepository;
use hoista_server::domain::{
    Provider, Secret,
    backup::{BACKUP_VERSION, Backup},
    domains::{
        CustomDomain, DomainDnsPurpose, DomainDnsRecord, DomainDnsRecordType, DomainObservation,
        DomainReadiness,
    },
    model::{
        Account, AccountStatus, Agent, AgentGrant, AgentTransport, Approval, ApprovalDecision,
        Deploy, DeployChannel, DeployFailure, DeployStatus, DeployStep, DiscoverySummary,
        Environment, GatedAction, Link, Mode, Plan, Project, Release, Resource, ResourceKind,
        ResourcePlan, ResourceRelease, SecretMetadata, Settings, Source, StepStatus, Usage,
        Webhook,
    },
};
use hoista_server::infrastructure::inspection::detect;
use std::path::Path;

/// The steps of a deploy, as the engine names them.
const STEPS: [&str; 9] = [
    "detect",
    "plan",
    "secrets",
    "leak_check",
    "build",
    "provision",
    "migrate",
    "release",
    "verify",
];
/// Days of request counts kept for each resource.
const USAGE_DAYS: i64 = 30;
/// How long an agent's pending request waits for an answer.
const APPROVAL_WAITS: Duration = Duration::hours(24);

/// Who started a deploy.
#[derive(Clone, Copy)]
enum By {
    Person(DeployChannel),
    Agent(&'static str),
}

/// How a deploy ended.
#[derive(Clone, Copy)]
enum Outcome {
    Live,
    /// Failed at a step: the error code, what it said, the variable it missed.
    Failed(
        &'static str,
        &'static str,
        &'static str,
        Option<&'static str>,
    ),
    /// Stopped by the person while this step ran.
    Stopped(&'static str),
}

/// One deploy in a project's history, oldest first.
struct Past {
    project: &'static str,
    title: &'static str,
    by: By,
    outcome: Outcome,
    minutes_ago: i64,
    seconds: i64,
}

const HOUR: i64 = 60;
const DAY: i64 = 24 * HOUR;

const HISTORY: &[Past] = &[
    Past {
        project: "sam-blog",
        title: "Post: Shipping on your own keys",
        by: By::Agent("agent-claude-code"),
        outcome: Outcome::Live,
        minutes_ago: 3 * DAY,
        seconds: 31,
    },
    Past {
        project: "recipe-swap",
        title: "First version",
        by: By::Person(DeployChannel::Cli),
        outcome: Outcome::Live,
        minutes_ago: 40 * DAY,
        seconds: 22,
    },
    Past {
        project: "recipe-swap",
        title: "Search by ingredient",
        by: By::Agent("agent-claude-code"),
        outcome: Outcome::Live,
        minutes_ago: 33 * DAY,
        seconds: 18,
    },
    Past {
        project: "short-links",
        title: "Track clicks per link",
        by: By::Agent("agent-claude-code"),
        outcome: Outcome::Live,
        minutes_ago: 6 * HOUR,
        seconds: 24,
    },
    Past {
        project: "habit-coach",
        title: "Weekly streak emails",
        by: By::Person(DeployChannel::Desktop),
        outcome: Outcome::Live,
        minutes_ago: 9 * DAY,
        seconds: 64,
    },
    Past {
        project: "image-resizer",
        title: "WebP by default",
        by: By::Person(DeployChannel::Desktop),
        outcome: Outcome::Live,
        minutes_ago: 5 * DAY,
        seconds: 22,
    },
    Past {
        project: "status-page",
        title: "Show incident history",
        by: By::Agent("agent-codex"),
        outcome: Outcome::Live,
        minutes_ago: 2 * DAY,
        seconds: 29,
    },
    Past {
        project: "support-chat",
        title: "Typing indicator",
        by: By::Agent("agent-codex"),
        outcome: Outcome::Live,
        minutes_ago: 9 * HOUR,
        seconds: 58,
    },
    Past {
        project: "invoice-bot",
        title: "Retry failed Stripe webhooks",
        by: By::Agent("agent-cursor"),
        outcome: Outcome::Live,
        minutes_ago: DAY,
        seconds: 52,
    },
    Past {
        project: "waitlist-v2",
        title: "First deploy",
        by: By::Person(DeployChannel::Desktop),
        outcome: Outcome::Live,
        minutes_ago: 6 * DAY,
        seconds: 21,
    },
    Past {
        project: "waitlist-v2",
        title: "Add referral codes",
        by: By::Agent("agent-claude-code"),
        outcome: Outcome::Failed(
            "secrets",
            "deploy.secret_missing",
            "The project needs RESEND_API_KEY, and it is not set.",
            Some("RESEND_API_KEY"),
        ),
        minutes_ago: 3 * HOUR,
        seconds: 5,
    },
    Past {
        project: "waitlist-v2",
        title: "Send welcome email",
        by: By::Agent("agent-claude-code"),
        outcome: Outcome::Failed(
            "secrets",
            "deploy.secret_missing",
            "The project needs RESEND_API_KEY, and it is not set.",
            Some("RESEND_API_KEY"),
        ),
        minutes_ago: 40,
        seconds: 6,
    },
    Past {
        project: "moodboard",
        title: "Add uploads",
        by: By::Person(DeployChannel::Cli),
        outcome: Outcome::Live,
        minutes_ago: 2 * DAY + 5 * HOUR,
        seconds: 44,
    },
    Past {
        project: "moodboard",
        title: "Try dark header",
        by: By::Agent("agent-hermes"),
        outcome: Outcome::Stopped("build"),
        minutes_ago: 2 * DAY + 2 * HOUR,
        seconds: 6,
    },
    Past {
        project: "moodboard",
        title: "Try new layout",
        by: By::Agent("agent-codex"),
        outcome: Outcome::Failed(
            "build",
            "build.failed",
            "The build failed: src/Layout.tsx imports a file that does not exist.",
            None,
        ),
        minutes_ago: 2 * DAY,
        seconds: 19,
    },
    Past {
        project: "moodboard",
        title: "Fix upload size limit",
        by: By::Person(DeployChannel::Desktop),
        outcome: Outcome::Live,
        minutes_ago: DAY,
        seconds: 41,
    },
    Past {
        project: "moodboard",
        title: "Add share button",
        by: By::Agent("agent-claude-code"),
        outcome: Outcome::Live,
        minutes_ago: 2 * HOUR,
        seconds: 38,
    },
];

/// The agents Sam has let in: id, name, how it connects, when it was last seen.
const AGENTS: &[(&str, &str, AgentTransport, i64)] = &[
    (
        "agent-claude-code",
        "Claude Code",
        AgentTransport::Stdio,
        2 * HOUR,
    ),
    ("agent-codex", "Codex", AgentTransport::Stdio, 40),
    ("agent-cursor", "Cursor", AgentTransport::Stdio, 3 * DAY),
    ("agent-chatgpt", "ChatGPT", AgentTransport::Link, 1),
    ("agent-hermes", "Hermes", AgentTransport::Link, 18),
];

/// Variables set in Hoista: project, the resource it was set from, name,
/// set by, minutes ago.
const SECRETS: &[(&str, &str, &str, &str, i64)] = &[
    (
        "moodboard",
        "moodboard-web",
        "DATABASE_URL",
        "person",
        30 * DAY,
    ),
    (
        "moodboard",
        "moodboard-web",
        "SESSION_SECRET",
        "person",
        30 * DAY,
    ),
    (
        "moodboard",
        "moodboard-web",
        "UPLOAD_LIMIT_MB",
        "agent-claude-code",
        DAY,
    ),
    (
        "moodboard",
        "moodboard-web",
        "SENTRY_DSN",
        "person",
        12 * DAY,
    ),
    ("waitlist-v2", "waitlist-web", "SITE_URL", "person", 6 * DAY),
    (
        "invoice-bot",
        "invoice-bot-web",
        "DATABASE_URL",
        "person",
        45 * DAY,
    ),
    (
        "invoice-bot",
        "invoice-bot-web",
        "STRIPE_WEBHOOK_SECRET",
        "agent-cursor",
        20 * DAY,
    ),
    (
        "habit-coach",
        "habit-coach-web",
        "DATABASE_URL",
        "person",
        50 * DAY,
    ),
    (
        "support-chat",
        "support-chat-web",
        "DATABASE_URL",
        "person",
        38 * DAY,
    ),
    (
        "support-chat",
        "support-chat-web",
        "OPENAI_API_KEY",
        "agent-codex",
        9 * DAY,
    ),
    (
        "short-links",
        "short-links-web",
        "ADMIN_TOKEN",
        "person",
        64 * DAY,
    ),
];

/// The person's own domains, on the resource that serves each.
const DOMAINS: &[(&str, &str, &str)] = &[
    ("moodboard", "moodboard-web", "boards.sam.dev"),
    ("invoice-bot", "invoice-bot-web", "invoices.sam.dev"),
    ("short-links", "short-links-web", "go.sam.dev"),
    ("status-page", "status-page-web", "status.sam.dev"),
    ("image-resizer", "image-resizer-web", "img.sam.dev"),
    ("sam-blog", "sam-blog-web", "sam.dev"),
];

/// The activity timeline: minutes ago, actor, project, kind, what happened.
const TIMELINE: &[(i64, &str, Option<&str>, &str, &str)] = &[
    (
        92 * DAY,
        "person",
        None,
        "account.updated",
        "Connected Cloudflare.",
    ),
    (
        60 * DAY,
        "person",
        None,
        "account.updated",
        "Connected Fly.io and Neon.",
    ),
    (
        30 * DAY,
        "person",
        Some("moodboard"),
        "app.created",
        "Created the project in Hoista.",
    ),
    (
        8 * DAY,
        "agent-claude-code",
        Some("waitlist-v2"),
        "app.created",
        "Created the project in Hoista.",
    ),
    (
        4 * DAY,
        "person",
        Some("portfolio-api"),
        "app.created",
        "Brought in portfolio-api from Railway to watch.",
    ),
    (
        2 * DAY + 2 * HOUR,
        "agent-hermes",
        Some("moodboard"),
        "deploy.cancel_requested",
        "Stopped a deploy: Try dark header.",
    ),
    (
        2 * DAY,
        "agent-codex",
        Some("moodboard"),
        "deploy.finished",
        "Deploy failed: Try new layout.",
    ),
    (
        DAY,
        "person",
        Some("moodboard"),
        "deploy.finished",
        "Deployed v3: Fix upload size limit.",
    ),
    (
        DAY,
        "person",
        None,
        "account.updated",
        "Connected Supabase.",
    ),
    (
        DAY - 2 * HOUR,
        "agent-claude-code",
        Some("moodboard"),
        "secret.updated",
        "Set UPLOAD_LIMIT_MB on moodboard.",
    ),
    (
        9 * HOUR,
        "agent-codex",
        Some("support-chat"),
        "deploy.finished",
        "Deployed v18: Typing indicator.",
    ),
    (
        6 * HOUR,
        "agent-claude-code",
        Some("short-links"),
        "deploy.finished",
        "Deployed v6: Track clicks per link.",
    ),
    (
        5 * HOUR,
        "engine",
        Some("moodboard"),
        "resource.updated",
        "25 uploads to moodboard were refused as too large.",
    ),
    (
        3 * HOUR,
        "agent-claude-code",
        Some("waitlist-v2"),
        "deploy.finished",
        "Deploy failed: RESEND_API_KEY isn't set.",
    ),
    (
        2 * HOUR,
        "agent-claude-code",
        Some("moodboard"),
        "deploy.finished",
        "Deployed v4: Add share button.",
    ),
    (
        2 * HOUR,
        "engine",
        None,
        "account.updated",
        "Vercel says the token for Sam's projects stopped working.",
    ),
    (
        40,
        "agent-claude-code",
        Some("waitlist-v2"),
        "deploy.finished",
        "Deploy failed: RESEND_API_KEY isn't set.",
    ),
    (
        12,
        "engine",
        Some("cron-worker"),
        "resource.updated",
        "Railway finished a deploy of cron-worker.",
    ),
];

fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn ago(now: DateTime<Utc>, minutes: i64) -> String {
    stamp(now - Duration::minutes(minutes))
}

/// Requests a resource served on `day` (`YYYY-MM-DD`), a little different each day.
pub fn usage(resource_id: &str, resource: &world::Resource, day: &str) -> Usage {
    let variation = day.bytes().fold(0_u32, |sum, byte| {
        sum.wrapping_mul(31).wrapping_add(u32::from(byte))
    });
    let requests = resource.kind.runs_code().then(|| {
        let swing = resource.daily_requests / 5;
        i64::from(resource.daily_requests - swing / 2 + variation % (swing + 1))
    });
    Usage {
        resource_id: resource_id.to_owned(),
        day: day.to_owned(),
        requests,
        errors: requests.map(|count| count / 400),
        compute_seconds: requests.map(|count| count / 20),
        storage_bytes: (resource.kind == ResourceKind::Database).then_some(212_000_000),
        egress_bytes: requests.map(|count| count * 18_000),
    }
}

/// The source of a project, as Hoista records it.
fn source(project: &world::Project, state: &Path) -> Source {
    match project.code {
        world::Code::Folder(name) => Source::Folder {
            path: code_folder(state).join(name).to_string_lossy().into_owned(),
        },
        world::Code::Github { repository, commit } => Source::Github {
            repository: repository.to_owned(),
            commit: commit.to_owned(),
        },
    }
}

/// The folder a project's code is read from.
fn checkout(project: &world::Project, state: &Path) -> Option<std::path::PathBuf> {
    let name = match project.code {
        world::Code::Folder(name) => name,
        world::Code::Github { repository, .. } => repository.rsplit('/').next()?,
    };
    Some(code_folder(state).join(name))
}

/// A small but real project folder: enough for Hoista to read how it deploys.
fn write_code(project: &world::Project, folder: &Path) -> std::io::Result<()> {
    let runs_service = world::RESOURCES.iter().any(|resource| {
        resource.project == Some(project.id) && resource.kind == ResourceKind::Service
    });
    let database = world::LINKS.iter().any(|link| {
        world::RESOURCES
            .iter()
            .any(|resource| resource.id == link.from && resource.project == Some(project.id))
            && link.via == "DATABASE_URL"
    });
    let mut dependencies = vec![if runs_service {
        r#""express": "^5.1.0""#
    } else {
        r#""hono": "^4.7.0""#
    }];
    if database {
        dependencies.push(r#""@neondatabase/serverless": "^1.0.0""#);
    }
    let variables: Vec<&str> = SECRETS
        .iter()
        .filter(|(owner, ..)| *owner == project.id)
        .map(|(_, _, name, ..)| *name)
        .chain((project.id == "waitlist-v2").then_some("RESEND_API_KEY"))
        .collect();
    std::fs::create_dir_all(folder.join("src"))?;
    std::fs::write(
        folder.join("package.json"),
        format!(
            "{{\n  \"name\": \"{}\",\n  \"private\": true,\n  \"scripts\": {{ \"build\": \"tsc\" }},\n  \"dependencies\": {{ {} }}\n}}\n",
            project.name,
            dependencies.join(", ")
        ),
    )?;
    let reads: String = variables
        .iter()
        .map(|name| format!("const {} = process.env.{name};\n", name.to_lowercase()))
        .collect();
    std::fs::write(
        folder.join("src").join("index.ts"),
        format!("// {}\n{reads}", project.name),
    )?;
    std::fs::write(
        folder.join("README.md"),
        format!(
            "# {}\n\nOne of Sam Rivera's example projects in the Hoista demo.\n",
            project.name
        ),
    )
}

fn plan(project: &world::Project, state: &Path) -> Option<Plan> {
    let detected = detect(&checkout(project, state)?).ok()?;
    // A D1 database is the Worker's binding, not a database a deploy provides.
    let ours: Vec<&world::Resource> = world::RESOURCES
        .iter()
        .filter(|resource| resource.project == Some(project.id))
        .filter(|resource| !resource.reference.starts_with("d1:"))
        .collect();
    let named = |kind: ResourceKind| -> Vec<String> {
        ours.iter()
            .filter(|resource| resource.kind == kind)
            .map(|resource| resource.name.to_owned())
            .collect()
    };
    // Everything waits for the database; code also waits for its buckets.
    let databases = named(ResourceKind::Database);
    let buckets = named(ResourceKind::Bucket);
    let resources = ours
        .iter()
        .map(|resource| ResourcePlan {
            schedules: None,
            provider_ref: Some(resource.reference.to_owned()),
            name: resource.name.to_owned(),
            kind: resource.kind,
            provider: provider_of(resource),
            account_id: resource.account.to_owned(),
            region: resource.region.map(str::to_owned),
            depends_on: match resource.kind {
                ResourceKind::Database => Vec::new(),
                kind if kind.runs_code() => [databases.clone(), buckets.clone()].concat(),
                _ => databases.clone(),
            },
        })
        .collect();
    Some(Plan {
        revision: 1,
        id: format!("plan-{}", project.id),
        source: source(project, state),
        framework: detected.framework,
        package_manager: detected.package_manager,
        build_command: detected.build_command,
        output_directory: detected.output_directory,
        env_names: detected.env_names,
        generated_secret_names: detected.generated_secret_names,
        prisma_migration: detected.prisma_migration,
        resources,
        warnings: detected.warnings,
    })
}

fn provider_of(resource: &world::Resource) -> Provider {
    world::account(resource.account).map_or(Provider::Cloudflare, |account| account.provider)
}

fn steps(outcome: Outcome) -> Vec<DeployStep> {
    let stop_at = match outcome {
        Outcome::Live => None,
        Outcome::Failed(step, ..) | Outcome::Stopped(step) => Some(step),
    };
    let mut reached = false;
    STEPS
        .iter()
        .map(|name| {
            let status = if reached {
                StepStatus::Pending
            } else if Some(*name) == stop_at {
                reached = true;
                StepStatus::Failed
            } else if *name == "migrate" {
                StepStatus::Skipped
            } else {
                StepStatus::Succeeded
            };
            DeployStep {
                name: (*name).to_owned(),
                status,
                message: None,
            }
        })
        .collect()
}

fn deploys(now: DateTime<Utc>, state: &Path) -> Vec<Deploy> {
    let mut deploys = Vec::new();
    for project in world::PROJECTS {
        let mut version = 0;
        let mut released = 0;
        for past in HISTORY.iter().filter(|past| past.project == project.id) {
            version += 1;
            let live = matches!(past.outcome, Outcome::Live);
            if live {
                released += 1;
            }
            let started = now - Duration::minutes(past.minutes_ago);
            let finished = started + Duration::seconds(past.seconds);
            let ours = world::RESOURCES
                .iter()
                .filter(|resource| resource.project == Some(project.id));
            let releases = if live {
                ours.clone()
                    .filter(|resource| resource.kind.runs_code())
                    .map(|resource| ResourceRelease {
                        resource_id: resource.id.to_owned(),
                        name: resource.name.to_owned(),
                        release: Release {
                            schedules: None,
                            reference: resource.reference.to_owned(),
                            version: format!("{}@{released}", resource.reference),
                            url: resource.url.map(str::to_owned),
                        },
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let (started_by, started_by_name, channel) = match past.by {
                By::Person(channel) => ("person".to_owned(), None, channel),
                By::Agent(agent) => (
                    agent.to_owned(),
                    AGENTS
                        .iter()
                        .find(|(id, ..)| *id == agent)
                        .map(|(_, name, ..)| (*name).to_owned()),
                    DeployChannel::Mcp,
                ),
            };
            let failure = match past.outcome {
                Outcome::Failed(step, code, message, missing) => Some(DeployFailure {
                    provider_response: None,
                    missing_secrets: missing
                        .map(|name| vec![name.to_owned()])
                        .unwrap_or_default(),
                    code: code.to_owned(),
                    message: message.to_owned(),
                    steps: vec![step.to_owned()],
                    release_started: false,
                    compensation_failed: false,
                }),
                _ => None,
            };
            deploys.push(Deploy {
                failure,
                environment_keys: Vec::new(),
                title: Some(past.title.to_owned()),
                channel,
                started_by_name,
                release_version: live.then_some(released),
                provenance: None,
                source_observation: None,
                cancel_requested: matches!(past.outcome, Outcome::Stopped(_)),
                plan_id: format!("plan-{}", project.id),
                releases,
                id: format!("dep-{}-{version}", project.id),
                project_id: project.id.to_owned(),
                version,
                source: source(project, state),
                started_by,
                status: if live {
                    DeployStatus::Live
                } else {
                    DeployStatus::Failed
                },
                steps: steps(past.outcome),
                url: if live {
                    ours.clone()
                        .find_map(|resource| resource.url)
                        .map(str::to_owned)
                } else {
                    None
                },
                started_at: stamp(started),
                finished_at: Some(stamp(finished)),
                duration_ms: Some(past.seconds * 1000),
                rollback_of: None,
            });
        }
    }
    deploys
}

fn backup(now: DateTime<Utc>, state: &Path) -> Backup {
    let accounts = world::ACCOUNTS
        .iter()
        .map(|account| Account {
            last_discovery: Some(DiscoverySummary {
                observed_at: ago(now, 15),
                complete: true,
                observed_resources: u32::try_from(world::resources_of(account.id).count()).ok(),
                warnings: Vec::new(),
            }),
            id: account.id.to_owned(),
            provider: account.provider,
            label: account.label.to_owned(),
            external_id: account.external_id.to_owned(),
            scopes: account
                .scopes
                .iter()
                .map(|scope| (*scope).to_owned())
                .collect(),
            status: if account.expired {
                AccountStatus::Expired
            } else {
                AccountStatus::Connected
            },
            connected_at: ago(now, account.connected_days_ago * DAY),
            credential: None,
            in_1password: account.in_1password,
        })
        .collect();
    let projects = world::PROJECTS
        .iter()
        .map(|project| Project {
            id: project.id.to_owned(),
            name: project.name.to_owned(),
            source: source(project, state),
            mode: project.mode,
            created_at: ago(now, project.created_days_ago * DAY),
            environment_id: project.environment.to_owned(),
        })
        .collect();
    let resources = world::RESOURCES
        .iter()
        .map(|resource| Resource {
            id: resource.id.to_owned(),
            project_id: resource.project.map(str::to_owned),
            kind: resource.kind,
            provider: provider_of(resource),
            account_id: resource.account.to_owned(),
            provider_ref: resource.reference.to_owned(),
            name: resource.name.to_owned(),
            region: resource.region.map(str::to_owned),
            status: resource.status,
            mode: resource
                .project
                .and_then(world::project)
                .map_or(Mode::Watching, |project| project.mode),
            url: resource.url.map(str::to_owned),
            group: resource.group.map(str::to_owned),
        })
        .collect();
    let links = world::LINKS
        .iter()
        .map(|link| Link {
            id: format!("link-{}-{}", link.from, link.to),
            from_resource: link.from.to_owned(),
            to_resource: link.to.to_owned(),
            via: link.via.to_owned(),
            derived: false,
            missing: false,
        })
        .collect();
    let custom_domains = DOMAINS
        .iter()
        .map(|(project, resource, hostname)| {
            let labels: Vec<&str> = hostname.split('.').collect();
            let zone = labels[labels.len().saturating_sub(2)..].join(".");
            CustomDomain {
                id: format!("domain-{hostname}"),
                project_id: (*project).to_owned(),
                resource_id: (*resource).to_owned(),
                hostname: (*hostname).to_owned(),
                observed: DomainObservation {
                    zone_id: format!("zone-{zone}"),
                    zone_name: zone,
                    nameservers: vec![
                        "ada.ns.cloudflare.com".into(),
                        "bob.ns.cloudflare.com".into(),
                    ],
                    registrar: Some("Cloudflare".into()),
                    dns: DomainReadiness::Active,
                    https: DomainReadiness::Active,
                    provider_domain_id: Some(format!("domain-{hostname}")),
                    certificate_id: Some(format!("cert-{hostname}")),
                    dns_records: vec![DomainDnsRecord {
                        name: (*hostname).to_owned(),
                        record_type: DomainDnsRecordType::CNAME,
                        value: "edge.example.net".into(),
                        purpose: DomainDnsPurpose::Routing,
                    }],
                    warnings: Vec::new(),
                },
                checked_at: ago(now, 30),
            }
        })
        .collect();
    Backup {
        version: BACKUP_VERSION,
        accounts,
        projects,
        resources,
        links,
        custom_domains,
        settings: Settings::default(),
        environments: vec![Environment {
            id: world::ENV_PRODUCTION.to_owned(),
            name: "production".into(),
            created_at: ago(now, 80 * DAY),
        }],
    }
}

fn agents(now: DateTime<Utc>) -> Vec<Agent> {
    AGENTS
        .iter()
        .map(|(id, name, transport, seen)| Agent {
            id: (*id).to_owned(),
            name: (*name).to_owned(),
            transport: *transport,
            scopes: vec![
                "apps:read".into(),
                "apps:write".into(),
                "apps:create".into(),
            ],
            last_seen: ago(now, *seen),
            revoked: false,
            grant: AgentGrant {
                projects: (*id == "agent-hermes").then(|| vec!["portfolio-api".into()]),
                environment_ids: None,
                deny: Vec::new(),
                expires_at: None,
                version: 1,
                without_asking: *id == "agent-claude-code",
                environment_rules: std::collections::BTreeMap::new(),
                exceptions: Vec::new(),
            },
        })
        .collect()
}

/// Writes the demo world into `repository`, whose state folder is `state`.
///
/// # Errors
/// Reports a store that refused a record, or a code folder that couldn't be written.
pub async fn seed(
    repository: &dyn CatalogRepository,
    vault: &dyn hoista_server::application::ports::vault::TokenVault,
    state: &Path,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    for project in world::PROJECTS {
        if let Some(folder) = checkout(project, state) {
            write_code(project, &folder)?;
        }
    }
    repository
        .restore(&backup(now, state))
        .await
        .context("catalog")?;
    for account in world::ACCOUNTS {
        vault
            .write(
                &format!("account/{}", account.id),
                Secret::new(account.id.to_owned()),
            )
            .await?;
    }
    for agent in agents(now) {
        repository.put_agent(&agent).await.context("agent")?;
    }
    for project in world::PROJECTS
        .iter()
        .filter(|project| project.mode == Mode::Managed)
    {
        if let Some(plan) = plan(project, state) {
            repository
                .put_plan(&plan)
                .await
                .with_context(|| format!("plan {}", plan.id))?;
        }
    }
    // A deploy is recorded as queued first, then finished, as the worker does.
    for deploy in deploys(now, state) {
        let queued = Deploy {
            status: DeployStatus::Queued,
            ..deploy.clone()
        };
        repository
            .enqueue(&queued)
            .await
            .with_context(|| format!("deploy {}", deploy.id))?;
        repository
            .put_deploy(&deploy)
            .await
            .with_context(|| format!("deploy {}", deploy.id))?;
    }
    // A variable reaches everything in the project that runs or serves,
    // so each of those holds it.
    for (project, _, name, by, minutes) in SECRETS {
        let holders = world::RESOURCES.iter().filter(|resource| {
            resource.project == Some(*project) && resource.kind != ResourceKind::Database
        });
        for resource in holders {
            repository
                .put_secret(&SecretMetadata {
                    requires_source_sync: false,
                    set_by: Some((*by).to_owned()),
                    managed_by: None,
                    project_id: (*project).to_owned(),
                    resource_id: resource.id.to_owned(),
                    name: (*name).to_owned(),
                    fingerprint: format!("{:064x}", name.len() * 7919),
                    set_at: ago(now, *minutes),
                })
                .await?;
        }
    }
    for resource in world::RESOURCES
        .iter()
        .filter(|resource| resource.project.is_some())
    {
        for day in 1..=USAGE_DAYS {
            let day = (now - Duration::days(day)).format("%Y-%m-%d").to_string();
            repository
                .put_usage(&usage(resource.id, resource, &day))
                .await?;
        }
    }
    repository
        .put_approval(&Approval {
            id: "approval-volume-delete".into(),
            agent_id: "agent-hermes".into(),
            action: GatedAction::ProviderChange,
            target: "portfolio-api-db:volumeDelete:pgdata".into(),
            deletion: None,
            reason: "The old pgdata volume is full of last year's logs; I'd like to remove it."
                .into(),
            created_at: ago(now, 6),
            expires_at: stamp(now + APPROVAL_WAITS),
            decision: ApprovalDecision::Pending,
        })
        .await?;
    repository
        .put_webhook(
            &Webhook {
                id: "webhook-hermes".into(),
                url: "https://hooks.example.com/hermes/hoista".into(),
                events: vec!["deploy.failed".into(), "resource.down".into()],
                project_id: None,
                created_at: ago(now, 10 * DAY),
            },
            0,
        )
        .await?;
    Ok(())
}

/// One entry of the demo's activity.
pub struct Entry {
    pub at: String,
    pub actor: &'static str,
    pub project: Option<&'static str>,
    pub kind: &'static str,
    pub summary: String,
    /// What an agent sent and how its call ended, as Hoista records a tool call.
    pub detail: Option<String>,
}

/// An agent's tool call, as Hoista records it.
struct Call {
    minutes_ago: i64,
    agent: &'static str,
    project: Option<&'static str>,
    tool: &'static str,
    /// What the agent sent, as JSON.
    sent: &'static str,
    outcome: &'static str,
    message: &'static str,
}

const CALLS: &[Call] = &[
    Call {
        minutes_ago: 3,
        agent: "agent-claude-code",
        project: Some("moodboard"),
        tool: "plan_deploy",
        sent: r#"{"project_id":"moodboard"}"#,
        outcome: "completed",
        message: "Planned a deploy of 3 resources on Cloudflare and Neon.",
    },
    Call {
        minutes_ago: 17,
        agent: "agent-chatgpt",
        project: None,
        tool: "list_projects",
        sent: "{}",
        outcome: "completed",
        message: "13 projects.",
    },
    Call {
        minutes_ago: 18,
        agent: "agent-hermes",
        project: Some("portfolio-api"),
        tool: "get_provider_details",
        sent: r#"{"resource_id":"portfolio-api-web"}"#,
        outcome: "completed",
        message: "Railway reports 6 deployments.",
    },
    Call {
        minutes_ago: 40,
        agent: "agent-claude-code",
        project: Some("waitlist-v2"),
        tool: "deploy",
        sent: r#"{"project_id":"waitlist-v2"}"#,
        outcome: "rejected",
        message: "The project needs RESEND_API_KEY, and it is not set.",
    },
    Call {
        minutes_ago: 61,
        agent: "agent-codex",
        project: Some("moodboard"),
        tool: "get_logs",
        sent: r#"{"project_id":"moodboard","since":"1h"}"#,
        outcome: "completed",
        message: "Read 200 lines.",
    },
    Call {
        minutes_ago: 2 * HOUR,
        agent: "agent-claude-code",
        project: Some("moodboard"),
        tool: "deploy",
        sent: r#"{"project_id":"moodboard","title":"Add share button"}"#,
        outcome: "completed",
        message: "Live · 38s",
    },
    Call {
        minutes_ago: 5 * HOUR,
        agent: "agent-claude-code",
        project: Some("moodboard"),
        tool: "get_project_map",
        sent: r#"{"project_id":"moodboard"}"#,
        outcome: "completed",
        message: "4 resources, 3 links.",
    },
    Call {
        minutes_ago: 18 * HOUR,
        agent: "agent-cursor",
        project: Some("invoice-bot"),
        tool: "get_usage",
        sent: r#"{"project_id":"invoice-bot"}"#,
        outcome: "completed",
        message: "8,930 requests in 7 days.",
    },
    Call {
        minutes_ago: DAY - 2 * HOUR,
        agent: "agent-claude-code",
        project: Some("moodboard"),
        tool: "set_secret",
        sent: r#"{"project_id":"moodboard","name":"UPLOAD_LIMIT_MB"}"#,
        outcome: "completed",
        message: "Set UPLOAD_LIMIT_MB.",
    },
    Call {
        minutes_ago: DAY + 6 * HOUR,
        agent: "agent-codex",
        project: Some("short-links"),
        tool: "rollback",
        sent: r#"{"project_id":"short-links","to":"v5"}"#,
        outcome: "completed",
        message: "Rolled back to v5.",
    },
    Call {
        minutes_ago: 2 * DAY,
        agent: "agent-hermes",
        project: Some("portfolio-api"),
        tool: "run_provider_operation",
        sent: r#"{"resource_id":"portfolio-api-web","name":"deployments"}"#,
        outcome: "completed",
        message: "Read 6 deployments.",
    },
    Call {
        minutes_ago: 3 * DAY,
        agent: "agent-codex",
        project: Some("recipe-swap"),
        tool: "delete_project",
        sent: r#"{"project_id":"recipe-swap"}"#,
        outcome: "rejected",
        message: "Hoista asked you first; you declined.",
    },
];

/// The demo's activity timeline, in place of what seeding wrote.
pub fn timeline(now: DateTime<Utc>) -> Vec<Entry> {
    let happened = TIMELINE
        .iter()
        .map(|(minutes, actor, project, kind, summary)| Entry {
            at: ago(now, *minutes),
            actor,
            project: *project,
            kind,
            summary: (*summary).to_owned(),
            detail: None,
        });
    let calls = CALLS.iter().map(|call| Entry {
        at: ago(now, call.minutes_ago),
        actor: call.agent,
        project: call.project,
        kind: "agent.tool",
        summary: format!("{}: {}", call.tool, call.outcome),
        detail: Some(
            serde_json::json!({
                "tool": call.tool,
                "sent": serde_json::from_str::<serde_json::Value>(call.sent).unwrap_or_default(),
                "result": { "outcome": call.outcome, "message": call.message, "approval_id": null },
            })
            .to_string(),
        ),
    });
    let mut entries: Vec<Entry> = happened.chain(calls).collect();
    entries.sort_by(|a, b| a.at.cmp(&b.at));
    entries
}
