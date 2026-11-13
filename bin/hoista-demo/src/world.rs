//! The demo's world: Sam Rivera's accounts, projects and what runs on them.
//! The seed writes it into Hoista's state, and the demo providers answer
//! from it, so what the app shows and what the providers report agree.

use hoista_server::domain::{
    Provider,
    model::{Mode, ResourceKind, ResourceStatus},
};

/// A connected provider account.
pub struct Account {
    pub id: &'static str,
    pub provider: Provider,
    pub label: &'static str,
    /// The provider's id for the account, team or workspace.
    pub external_id: &'static str,
    pub scopes: &'static [&'static str],
    /// The token stopped working, so the "reconnect" flow can be seen.
    pub expired: bool,
    /// The token is kept in 1Password rather than the keychain.
    pub in_1password: bool,
    pub connected_days_ago: i64,
}

/// Where a project's code lives.
pub enum Code {
    /// A folder the seed makes under the demo's state folder, `code/<name>`.
    Folder(&'static str),
    /// A GitHub repository at one commit.
    Github {
        repository: &'static str,
        commit: &'static str,
    },
}

pub struct Project {
    pub id: &'static str,
    pub name: &'static str,
    pub mode: Mode,
    pub environment: &'static str,
    pub code: Code,
    pub created_days_ago: i64,
}

/// A resource at a provider. `project` is None for what Hoista found but
/// nobody has put in a project yet (the import screen's list).
pub struct Resource {
    pub id: &'static str,
    pub project: Option<&'static str>,
    pub account: &'static str,
    pub kind: ResourceKind,
    pub reference: &'static str,
    pub name: &'static str,
    pub region: Option<&'static str>,
    pub status: ResourceStatus,
    pub url: Option<&'static str>,
    /// The provider's grouping (a Railway or Supabase project), by name.
    pub group: Option<&'static str>,
    /// Requests on an ordinary day, for usage, traffic and charts.
    pub daily_requests: u32,
}

/// One line on a project map: `from` uses `to`.
pub struct Link {
    pub from: &'static str,
    pub to: &'static str,
    pub via: &'static str,
}

pub const ENV_PRODUCTION: &str = "env_production";

pub const ACCOUNTS: &[Account] = &[
    Account {
        id: "acc-cloudflare",
        provider: Provider::Cloudflare,
        label: "Sam's Account",
        external_id: "4f2b9c6d1e8a7b3c5d0e9f1a2b3c4d5e",
        scopes: &[
            "Workers Scripts",
            "Workers R2 Storage",
            "D1",
            "Zone",
            "DNS",
            "Workers Routes",
        ],
        expired: false,
        in_1password: false,
        connected_days_ago: 92,
    },
    Account {
        id: "acc-fly",
        provider: Provider::Fly,
        label: "sam-builds",
        external_id: "sam-builds",
        scopes: &["apps", "machines"],
        expired: false,
        in_1password: false,
        connected_days_ago: 60,
    },
    Account {
        id: "acc-neon",
        provider: Provider::Neon,
        label: "Sam",
        external_id: "org-quiet-river-28173",
        scopes: &["projects"],
        expired: false,
        in_1password: true,
        connected_days_ago: 60,
    },
    Account {
        id: "acc-github",
        provider: Provider::Github,
        label: "sam-builds",
        external_id: "sam-builds",
        scopes: &["contents:read"],
        expired: false,
        in_1password: false,
        connected_days_ago: 40,
    },
    Account {
        id: "acc-railway",
        provider: Provider::Railway,
        label: "Sam's workspace",
        external_id: "ws-sam-7f3e",
        scopes: &["read", "deploy"],
        expired: false,
        in_1password: false,
        connected_days_ago: 4,
    },
    Account {
        id: "acc-railway-team",
        provider: Provider::Railway,
        label: "Studio team",
        external_id: "ws-studio-2c91",
        scopes: &["read"],
        expired: false,
        in_1password: false,
        connected_days_ago: 4,
    },
    Account {
        id: "acc-supabase",
        provider: Provider::Supabase,
        label: "Sam's Org",
        external_id: "org-sam-rivera",
        scopes: &["projects:read", "database:read"],
        expired: false,
        in_1password: false,
        connected_days_ago: 1,
    },
    Account {
        id: "acc-runpod",
        provider: Provider::Runpod,
        label: "Runpod account",
        external_id: "user-sam-41a",
        scopes: &["read"],
        expired: false,
        in_1password: false,
        connected_days_ago: 12,
    },
    Account {
        id: "acc-vercel",
        provider: Provider::Vercel,
        label: "Sam's projects",
        external_id: "team_sam_projects",
        scopes: &["read"],
        expired: true,
        in_1password: false,
        connected_days_ago: 120,
    },
];

pub const PROJECTS: &[Project] = &[
    Project {
        id: "moodboard",
        name: "moodboard",
        mode: Mode::Managed,
        environment: hoista_server::domain::model::DEFAULT_ENVIRONMENT,
        code: Code::Folder("moodboard"),
        created_days_ago: 30,
    },
    Project {
        id: "invoice-bot",
        name: "invoice-bot",
        mode: Mode::Managed,
        environment: hoista_server::domain::model::DEFAULT_ENVIRONMENT,
        code: Code::Github {
            repository: "sam-builds/invoice-bot",
            commit: "9f4c2a7e1b6d3f8a0c5e2b7d4f1a6c3e8b0d5f2a",
        },
        created_days_ago: 45,
    },
    Project {
        id: "waitlist-v2",
        name: "waitlist-v2",
        mode: Mode::Managed,
        environment: hoista_server::domain::model::DEFAULT_ENVIRONMENT,
        code: Code::Folder("waitlist-v2"),
        created_days_ago: 8,
    },
    Project {
        id: "habit-coach",
        name: "habit-coach",
        mode: Mode::Managed,
        environment: hoista_server::domain::model::DEFAULT_ENVIRONMENT,
        code: Code::Folder("habit-coach"),
        created_days_ago: 50,
    },
    Project {
        id: "recipe-swap",
        name: "recipe-swap",
        mode: Mode::Managed,
        environment: hoista_server::domain::model::DEFAULT_ENVIRONMENT,
        code: Code::Folder("recipe-swap"),
        created_days_ago: 70,
    },
    Project {
        id: "portfolio-api",
        name: "portfolio-api",
        mode: Mode::Watching,
        environment: hoista_server::domain::model::DEFAULT_ENVIRONMENT,
        code: Code::Github {
            repository: "sam-builds/portfolio-api",
            commit: "713ba4f2c9e1d8b7a6f5e4d3c2b1a0f9e8d7c6b5",
        },
        created_days_ago: 4,
    },
    Project {
        id: "cron-worker",
        name: "cron-worker",
        mode: Mode::Watching,
        environment: hoista_server::domain::model::DEFAULT_ENVIRONMENT,
        code: Code::Github {
            repository: "sam-builds/cron-worker",
            commit: "c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0",
        },
        created_days_ago: 20,
    },
    Project {
        id: "short-links",
        name: "short-links",
        mode: Mode::Managed,
        environment: hoista_server::domain::model::DEFAULT_ENVIRONMENT,
        code: Code::Github {
            repository: "sam-builds/short-links",
            commit: "5be1f39d8c7a6b5e4f3d2c1b0a9f8e7d6c5b4a39",
        },
        created_days_ago: 64,
    },
    Project {
        id: "status-page",
        name: "status-page",
        mode: Mode::Managed,
        environment: ENV_PRODUCTION,
        code: Code::Folder("status-page"),
        created_days_ago: 75,
    },
    Project {
        id: "support-chat",
        name: "support-chat",
        mode: Mode::Managed,
        environment: hoista_server::domain::model::DEFAULT_ENVIRONMENT,
        code: Code::Github {
            repository: "sam-builds/support-chat",
            commit: "a81d7c0e5f4b3a2c1d0e9f8a7b6c5d4e3f2a1b0c",
        },
        created_days_ago: 38,
    },
    Project {
        id: "image-resizer",
        name: "image-resizer",
        mode: Mode::Managed,
        environment: hoista_server::domain::model::DEFAULT_ENVIRONMENT,
        code: Code::Folder("image-resizer"),
        created_days_ago: 55,
    },
    Project {
        id: "sam-blog",
        name: "sam-blog",
        mode: Mode::Managed,
        environment: ENV_PRODUCTION,
        code: Code::Github {
            repository: "sam-builds/blog",
            commit: "3f9a2c1b8e7d6f5a4c3b2e1d0f9a8c7b6e5d4f3a",
        },
        created_days_ago: 300,
    },
    Project {
        id: "old-portfolio",
        name: "old-portfolio",
        mode: Mode::Watching,
        environment: hoista_server::domain::model::DEFAULT_ENVIRONMENT,
        code: Code::Github {
            repository: "sam-builds/old-portfolio",
            commit: "0aa31f5c4b3a2d1e0f9c8b7a6d5e4f3c2b1a0d9e",
        },
        created_days_ago: 200,
    },
];

const fn resource(
    id: &'static str,
    project: Option<&'static str>,
    account: &'static str,
    kind: ResourceKind,
    reference: &'static str,
    name: &'static str,
) -> Resource {
    Resource {
        id,
        project,
        account,
        kind,
        reference,
        name,
        region: None,
        status: ResourceStatus::Live,
        url: None,
        group: None,
        daily_requests: 0,
    }
}

pub const RESOURCES: &[Resource] = &[
    // moodboard: a Worker, its nightly job, a Neon database and an R2 bucket.
    Resource {
        url: Some("https://moodboard.sam.workers.dev"),
        daily_requests: 1780,
        ..resource(
            "moodboard-web",
            Some("moodboard"),
            "acc-cloudflare",
            ResourceKind::Web,
            "worker:moodboard",
            "moodboard",
        )
    },
    resource(
        "moodboard-digest",
        Some("moodboard"),
        "acc-cloudflare",
        ResourceKind::Job,
        "worker:nightly-digest",
        "nightly-digest",
    ),
    Resource {
        region: Some("aws-us-east-2"),
        status: ResourceStatus::Sleeping,
        ..resource(
            "moodboard-db",
            Some("moodboard"),
            "acc-neon",
            ResourceKind::Database,
            "young-sun-41823",
            "moodboard-db",
        )
    },
    resource(
        "moodboard-uploads",
        Some("moodboard"),
        "acc-cloudflare",
        ResourceKind::Bucket,
        "r2:default:moodboard-uploads",
        "moodboard-uploads",
    ),
    // invoice-bot: a Fly app, a Neon database and an R2 bucket for PDFs.
    Resource {
        region: Some("ams"),
        url: Some("https://invoices.sam.dev"),
        daily_requests: 1280,
        ..resource(
            "invoice-bot-web",
            Some("invoice-bot"),
            "acc-fly",
            ResourceKind::Service,
            "invoice-bot",
            "invoice-bot",
        )
    },
    Resource {
        region: Some("aws-eu-central-1"),
        ..resource(
            "invoice-bot-db",
            Some("invoice-bot"),
            "acc-neon",
            ResourceKind::Database,
            "bold-meadow-30517",
            "invoice-bot-db",
        )
    },
    resource(
        "invoice-bot-files",
        Some("invoice-bot"),
        "acc-cloudflare",
        ResourceKind::Bucket,
        "r2:default:invoice-pdfs",
        "invoice-pdfs",
    ),
    // waitlist-v2: its last deploys failed; v1 is still live.
    Resource {
        status: ResourceStatus::Failing,
        url: Some("https://waitlist-v2.sam.workers.dev"),
        daily_requests: 6,
        ..resource(
            "waitlist-web",
            Some("waitlist-v2"),
            "acc-cloudflare",
            ResourceKind::Web,
            "worker:waitlist-v2",
            "waitlist-v2",
        )
    },
    // habit-coach: asleep until someone opens it.
    Resource {
        region: Some("lhr"),
        status: ResourceStatus::Sleeping,
        url: Some("https://habit-coach.fly.dev"),
        daily_requests: 2,
        ..resource(
            "habit-coach-web",
            Some("habit-coach"),
            "acc-fly",
            ResourceKind::Service,
            "habit-coach",
            "habit-coach",
        )
    },
    Resource {
        region: Some("aws-eu-west-2"),
        status: ResourceStatus::Sleeping,
        ..resource(
            "habit-coach-db",
            Some("habit-coach"),
            "acc-neon",
            ResourceKind::Database,
            "calm-forest-77102",
            "habit-coach-db",
        )
    },
    // recipe-swap: nobody has opened it in weeks.
    Resource {
        url: Some("https://recipe-swap.sam.workers.dev"),
        ..resource(
            "recipe-swap-web",
            Some("recipe-swap"),
            "acc-cloudflare",
            ResourceKind::Web,
            "worker:recipe-swap",
            "recipe-swap",
        )
    },
    // portfolio-api: watched on Railway, with its auth on Supabase.
    Resource {
        region: Some("europe-west4"),
        url: Some("https://api.sam.dev"),
        group: Some("portfolio"),
        daily_requests: 2300,
        ..resource(
            "portfolio-api-web",
            Some("portfolio-api"),
            "acc-railway",
            ResourceKind::Service,
            "prj-portfolio/svc-portfolio-api",
            "portfolio-api",
        )
    },
    Resource {
        region: Some("europe-west4"),
        group: Some("portfolio"),
        ..resource(
            "portfolio-api-db",
            Some("portfolio-api"),
            "acc-railway",
            ResourceKind::Database,
            "prj-portfolio/svc-portfolio-db",
            "Postgres",
        )
    },
    Resource {
        region: Some("eu-west-3"),
        group: Some("portfolio-auth"),
        url: Some("https://qzvkpmsamrivera01.supabase.co"),
        ..resource(
            "portfolio-api-auth",
            Some("portfolio-api"),
            "acc-supabase",
            ResourceKind::Database,
            "qzvkpmsamrivera01",
            "portfolio-auth",
        )
    },
    Resource {
        region: Some("eu-west-3"),
        group: Some("portfolio-auth"),
        ..resource(
            "portfolio-api-avatars",
            Some("portfolio-api"),
            "acc-supabase",
            ResourceKind::Bucket,
            "qzvkpmsamrivera01/bucket:avatars",
            "avatars",
        )
    },
    // cron-worker: a scheduled Railway service.
    Resource {
        region: Some("europe-west4"),
        group: Some("cron-worker"),
        ..resource(
            "cron-worker-job",
            Some("cron-worker"),
            "acc-railway",
            ResourceKind::Job,
            "prj-cron/svc-cron-worker",
            "cron-worker",
        )
    },
    // short-links: a Worker with a D1 database.
    Resource {
        url: Some("https://go.sam.dev"),
        daily_requests: 3130,
        ..resource(
            "short-links-web",
            Some("short-links"),
            "acc-cloudflare",
            ResourceKind::Web,
            "worker:short-links",
            "short-links",
        )
    },
    resource(
        "short-links-db",
        Some("short-links"),
        "acc-cloudflare",
        ResourceKind::Database,
        "d1:3c8e1f2a-9b4d-4e6f-8a1c-2d3e4f5a6b7c",
        "short-links-db",
    ),
    // status-page, in the production environment.
    Resource {
        url: Some("https://status.sam.dev"),
        daily_requests: 460,
        ..resource(
            "status-page-web",
            Some("status-page"),
            "acc-cloudflare",
            ResourceKind::Web,
            "worker:status-page",
            "status-page",
        )
    },
    resource(
        "status-page-job",
        Some("status-page"),
        "acc-cloudflare",
        ResourceKind::Job,
        "worker:status-checks",
        "status-checks",
    ),
    // support-chat: two Fly machines and a Neon database.
    Resource {
        region: Some("lhr"),
        url: Some("https://support-chat.fly.dev"),
        daily_requests: 410,
        ..resource(
            "support-chat-web",
            Some("support-chat"),
            "acc-fly",
            ResourceKind::Service,
            "support-chat",
            "support-chat",
        )
    },
    Resource {
        region: Some("aws-eu-west-2"),
        ..resource(
            "support-chat-db",
            Some("support-chat"),
            "acc-neon",
            ResourceKind::Database,
            "steep-wave-55190",
            "support-chat-db",
        )
    },
    // image-resizer: a Worker and the bucket it writes to.
    Resource {
        url: Some("https://img.sam.dev"),
        daily_requests: 2190,
        ..resource(
            "image-resizer-web",
            Some("image-resizer"),
            "acc-cloudflare",
            ResourceKind::Web,
            "worker:image-resizer",
            "image-resizer",
        )
    },
    resource(
        "image-resizer-files",
        Some("image-resizer"),
        "acc-cloudflare",
        ResourceKind::Bucket,
        "r2:default:resized-images",
        "resized-images",
    ),
    // sam-blog, in the production environment.
    Resource {
        url: Some("https://sam.dev"),
        daily_requests: 730,
        ..resource(
            "sam-blog-web",
            Some("sam-blog"),
            "acc-cloudflare",
            ResourceKind::Web,
            "worker:sam-blog",
            "sam-blog",
        )
    },
    // old-portfolio: paused by Sam.
    Resource {
        region: Some("us-west2"),
        status: ResourceStatus::Paused,
        url: Some("https://old-portfolio.up.railway.app"),
        group: Some("old-portfolio"),
        ..resource(
            "old-portfolio-web",
            Some("old-portfolio"),
            "acc-railway",
            ResourceKind::Service,
            "prj-old/svc-old-portfolio",
            "old-portfolio",
        )
    },
    // Found on the providers and not in a project yet.
    Resource {
        region: Some("europe-west4"),
        group: Some("analytics"),
        ..resource(
            "found-analytics-db",
            None,
            "acc-railway",
            ResourceKind::Database,
            "prj-analytics/svc-analytics-db",
            "Postgres",
        )
    },
    Resource {
        region: Some("us-east4"),
        url: Some("https://shop.up.railway.app"),
        group: Some("shop"),
        ..resource(
            "found-shop-web",
            None,
            "acc-railway-team",
            ResourceKind::Web,
            "prj-shop/svc-shop-web",
            "web",
        )
    },
    Resource {
        region: Some("us-east4"),
        group: Some("shop"),
        ..resource(
            "found-shop-worker",
            None,
            "acc-railway-team",
            ResourceKind::Job,
            "prj-shop/svc-shop-worker",
            "worker",
        )
    },
    Resource {
        region: Some("us-east4"),
        group: Some("shop"),
        ..resource(
            "found-shop-redis",
            None,
            "acc-railway-team",
            ResourceKind::Database,
            "prj-shop/svc-shop-redis",
            "Redis",
        )
    },
    Resource {
        region: Some("us-east4"),
        group: Some("discord-bot"),
        ..resource(
            "found-discord-bot",
            None,
            "acc-railway-team",
            ResourceKind::Service,
            "prj-discord/svc-bot",
            "bot",
        )
    },
    Resource {
        region: Some("EU-RO-1"),
        url: Some("https://4c9d2e7f1a-8888.proxy.runpod.net"),
        ..resource(
            "found-llama-finetune",
            None,
            "acc-runpod",
            ResourceKind::Service,
            "pod:4c9d2e7f1a",
            "llama-finetune",
        )
    },
    Resource {
        url: Some("https://api.runpod.ai/v2/sdxl7q2k9w"),
        ..resource(
            "found-sdxl-api",
            None,
            "acc-runpod",
            ResourceKind::Service,
            "endpoint:sdxl7q2k9w",
            "sdxl-api",
        )
    },
    Resource {
        region: Some("EU-RO-1"),
        ..resource(
            "found-models",
            None,
            "acc-runpod",
            ResourceKind::Bucket,
            "volume:v8k2m4p6",
            "models",
        )
    },
];

pub const LINKS: &[Link] = &[
    Link {
        from: "moodboard-web",
        to: "moodboard-db",
        via: "DATABASE_URL",
    },
    Link {
        from: "moodboard-web",
        to: "moodboard-uploads",
        via: "UPLOADS",
    },
    Link {
        from: "moodboard-digest",
        to: "moodboard-db",
        via: "DATABASE_URL",
    },
    Link {
        from: "invoice-bot-web",
        to: "invoice-bot-db",
        via: "DATABASE_URL",
    },
    Link {
        from: "invoice-bot-web",
        to: "invoice-bot-files",
        via: "R2_BUCKET",
    },
    Link {
        from: "habit-coach-web",
        to: "habit-coach-db",
        via: "DATABASE_URL",
    },
    Link {
        from: "portfolio-api-web",
        to: "portfolio-api-db",
        via: "DATABASE_URL",
    },
    Link {
        from: "short-links-web",
        to: "short-links-db",
        via: "DB",
    },
    Link {
        from: "support-chat-web",
        to: "support-chat-db",
        via: "DATABASE_URL",
    },
    Link {
        from: "image-resizer-web",
        to: "image-resizer-files",
        via: "IMAGES",
    },
];

/// The account with `id`.
pub fn account(id: &str) -> Option<&'static Account> {
    ACCOUNTS.iter().find(|account| account.id == id)
}

/// The account a provider knows by `external_id`.
pub fn account_at(provider: Provider, external_id: &str) -> Option<&'static Account> {
    ACCOUNTS
        .iter()
        .find(|account| account.provider == provider && account.external_id == external_id)
}

/// What the provider has under one account.
pub fn resources_of(account: &str) -> impl Iterator<Item = &'static Resource> {
    RESOURCES
        .iter()
        .filter(move |resource| resource.account == account)
}

/// The resource the provider knows by `reference`.
pub fn resource_at(provider: Provider, reference: &str) -> Option<&'static Resource> {
    RESOURCES.iter().find(|resource| {
        resource.reference == reference
            && account(resource.account).is_some_and(|account| account.provider == provider)
    })
}

/// The project with `id`.
pub fn project(id: &str) -> Option<&'static Project> {
    PROJECTS.iter().find(|project| project.id == id)
}
