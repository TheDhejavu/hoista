//! Deploy planning from detected source facts. Deterministic, with no network access.

use crate::ports::catalog::EngineError;
use crate::{Provider, domain::model::*};
use std::collections::{BTreeMap, BTreeSet};

const MAX_PLAN_RESOURCES: usize = 32;

/// What the source inspector says when a folder holds no project it can deploy.
pub const NO_SUPPORTED_PROJECT: &str =
    "No supported application was detected. Add package.json, a server manifest or index.html.";

/// What Hoista found in a folder before it's connected to a project: nothing is
/// linked, built or deployed by looking.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    ts_rs::TS,
)]
pub struct SourcePreview {
    /// The folder, as given.
    pub path: String,
    /// The framework Hoista recognised, when it did.
    pub framework: Option<String>,
    /// Static, edge, job, service or supabase, when recognised.
    pub shape: Option<String>,
    /// From the lockfile.
    pub package_manager: Option<String>,
    /// The build command, when there is one.
    pub build_command: Vec<String>,
    /// Environment variable names the code reads; never values.
    pub env_names: Vec<String>,
    /// Things to know, in plain words.
    pub warnings: Vec<String>,
    /// Set when Hoista didn't recognise the project, saying why.
    pub unrecognised: Option<String>,
}

/// What source inspection found, as input to [`plan`].
#[derive(Debug, Clone)]
pub struct Detection {
    /// UTC cron schedules set in the source config; null leaves the provider's triggers as they are.
    pub schedules: Option<Vec<String>>,
    /// Detected framework name.
    pub framework: String,
    /// Static, edge, job, service or supabase.
    pub shape: String,
    /// Package manager from lockfiles.
    pub package_manager: Option<String>,
    /// Build executable and arguments.
    pub build_command: Vec<String>,
    /// Default build output directory.
    pub output_directory: String,
    /// Names only, never environment values.
    pub env_names: Vec<String>,
    /// Auth secret names Hoista may generate on the first deploy.
    pub generated_secret_names: Vec<String>,
    /// Checked-in migration metadata, without credentials.
    pub prisma_migration: Option<PrismaMigration>,
    /// Whether a new Postgres resource is needed.
    pub postgres: bool,
    /// Limitations found in the source.
    pub warnings: Vec<String>,
}

/// What each provider's adapter can do, in the order Hoista suggests providers.
/// Planning chooses from these and never checks a provider by name.
#[derive(Debug, Clone, Default)]
pub struct Providers(Vec<(Provider, Capabilities)>);

impl Providers {
    /// The providers, in the order Hoista suggests them.
    #[must_use]
    pub fn new(providers: Vec<(Provider, Capabilities)>) -> Self {
        Self(providers)
    }

    fn can(&self, provider: Provider, ability: fn(&Capabilities) -> bool) -> bool {
        self.0
            .iter()
            .any(|(known, capabilities)| *known == provider && ability(capabilities))
    }

    fn first(&self, ability: fn(&Capabilities) -> bool) -> Option<Provider> {
        self.0
            .iter()
            .find(|(_, capabilities)| ability(capabilities))
            .map(|(provider, _)| *provider)
    }

    /// The providers that can, by name: `Cloudflare or Fly.io`.
    fn names(&self, ability: fn(&Capabilities) -> bool) -> String {
        let names: Vec<_> = self
            .0
            .iter()
            .filter(|(_, capabilities)| ability(capabilities))
            .map(|(provider, _)| provider.display_name())
            .collect();
        match names.as_slice() {
            [] => "no provider".into(),
            [one] => (*one).into(),
            [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
        }
    }
}

fn deploys(capabilities: &Capabilities) -> bool {
    capabilities.plan_deploy
}

fn runs_services(capabilities: &Capabilities) -> bool {
    capabilities.plan_deploy && capabilities.containers
}

fn runs_cron(capabilities: &Capabilities) -> bool {
    capabilities.cron
}

fn holds_databases(capabilities: &Capabilities) -> bool {
    capabilities.project_database
}

/// Builds a plan for the pinned provider, else the project's current provider, else
/// the first of `providers` that can run it (one that runs containers for a
/// service).
///
/// # Errors
/// Fails if the project name is invalid, the provider can't take this source, or a needed
/// account is not connected.
pub fn plan(
    source: Source,
    name: &str,
    mut detected: Detection,
    accounts: &[Account],
    providers: &Providers,
    pinned: Option<Provider>,
    existing: Option<Provider>,
) -> Result<Plan, EngineError> {
    if !valid_slug(name) {
        return Err(EngineError::Invalid(
            "Use a lowercase project name containing letters, digits and hyphens.",
        ));
    }
    let service = detected.shape == "service";
    let default = providers.first(if service { runs_services } else { deploys });
    let Some(target) = pinned.or(existing).or(default) else {
        return Err(EngineError::Invalid(
            "No provider Hoista knows can build and deploy this project.",
        ));
    };
    if !providers.can(target, deploys) {
        return Err(EngineError::Unsupported(format!(
            "plan_deploy builds on {}. For a new project on {target}, use create_project_at_provider with the GitHub repository.",
            providers.names(deploys)
        )));
    }
    let containers = providers.can(target, runs_services);
    if service && !containers {
        return Err(EngineError::Unsupported(format!(
            "Long-running services require {} rather than {target}.",
            providers.names(runs_services)
        )));
    }
    if detected.schedules.is_some() && !providers.can(target, runs_cron) {
        return Err(EngineError::Unsupported(format!(
            "Exact UTC cron schedules require {}.",
            providers.names(runs_cron)
        )));
    }
    if detected.schedules.is_some() {
        detected
            .warnings
            .push("Cron schedules use UTC. Provider propagation can take up to 15 minutes; job health checks verify configuration without executing the scheduled handler.".into());
    }
    let account = |provider: Provider, missing: String| {
        accounts
            .iter()
            .find(|a| a.provider == provider && a.status == AccountStatus::Connected)
            .ok_or(EngineError::Unsupported(missing))
    };
    let mut resources = Vec::new();
    let mut dependencies = Vec::new();
    if detected.postgres {
        let db_name = format!("{name}-db");
        if !valid_slug(&db_name) {
            return Err(EngineError::Invalid(
                "The project name is too long for its database resource.",
            ));
        }
        let Some(database) = providers.first(holds_databases) else {
            return Err(EngineError::Invalid(
                "This project needs Postgres, and no provider Hoista knows creates it.",
            ));
        };
        let missing = format!(
            "This project needs Postgres, which Hoista creates on {database}. Connect a {database} account before planning it."
        );
        dependencies.push(db_name.clone());
        resources.push(ResourcePlan {
            schedules: None,
            provider_ref: None,
            name: db_name,
            kind: ResourceKind::Database,
            provider: database,
            account_id: account(database, missing)?.id.clone(),
            region: None,
            depends_on: vec![],
        });
    }
    resources.push(ResourcePlan {
        schedules: detected.schedules,
        provider_ref: None,
        name: name.into(),
        kind: if detected.shape == "job" {
            ResourceKind::Job
        } else if containers {
            ResourceKind::Service
        } else {
            ResourceKind::Web
        },
        provider: target,
        account_id: account(
            target,
            format!("Deploying this project needs a connected {target} account. Accounts on providers Hoista only watches can't be deployed to."),
        )?
        .id
        .clone(),
        region: None,
        depends_on: dependencies,
    });
    let plan = Plan {
        revision: 0,
        id: uuid::Uuid::new_v4().to_string(),
        source,
        framework: detected.framework,
        package_manager: detected.package_manager,
        build_command: detected.build_command,
        output_directory: detected.output_directory,
        env_names: detected.env_names,
        prisma_migration: detected.prisma_migration,
        generated_secret_names: detected.generated_secret_names,
        resources,
        warnings: detected.warnings,
    };
    Ok(plan)
}

/// Returns the plan's resource indexes in dependency order.
///
/// # Errors
/// Rejects an empty or oversized plan, invalid schedules, invalid or duplicate names,
/// missing dependencies and cycles.
pub fn release_order(
    resources: &[ResourcePlan],
    providers: &Providers,
) -> Result<Vec<usize>, EngineError> {
    if resources.is_empty() || resources.len() > MAX_PLAN_RESOURCES {
        return Err(EngineError::Invalid(
            "A plan needs between 1 and 32 resources.",
        ));
    }
    let mut indexes = BTreeMap::new();
    for resource in resources {
        if let Some(schedules) = &resource.schedules
            && (!providers.can(resource.provider, runs_cron)
                || !matches!(resource.kind, ResourceKind::Web | ResourceKind::Job)
                || !crate::domain::schedules::valid(schedules))
        {
            return Err(EngineError::Unsupported(format!(
                "Schedules require a web or job resource on {} and up to 100 distinct supported five-field UTC cron expressions.",
                providers.names(runs_cron)
            )));
        }
    }
    for (index, resource) in resources.iter().enumerate() {
        if !valid_slug(&resource.name) || indexes.insert(resource.name.as_str(), index).is_some() {
            return Err(EngineError::Invalid(
                "Resource names must be unique lowercase slugs.",
            ));
        }
    }
    let mut ordered = Vec::new();
    let mut done = BTreeSet::new();
    while ordered.len() < resources.len() {
        let previous = ordered.len();
        for (index, resource) in resources.iter().enumerate() {
            if done.contains(&index) {
                continue;
            }
            if resource
                .depends_on
                .iter()
                .any(|name| !indexes.contains_key(name.as_str()))
            {
                return Err(EngineError::Invalid(
                    "A dependency refers to a missing resource.",
                ));
            }
            if resource.depends_on.iter().all(|name| {
                indexes
                    .get(name.as_str())
                    .is_some_and(|index| done.contains(index))
            }) {
                done.insert(index);
                ordered.push(index);
            }
        }
        if ordered.len() == previous {
            return Err(EngineError::Invalid(
                "The deployment plan contains a dependency cycle.",
            ));
        }
    }
    Ok(ordered)
}

/// Checks the plan's one `DATABASE_URL` database and where migrations run.
///
/// # Errors
/// Rejects ambiguous databases, missing dependencies and unsupported migration targets.
pub fn validate_database_plan(plan: &Plan, providers: &Providers) -> Result<(), EngineError> {
    let databases: Vec<_> = plan
        .resources
        .iter()
        .filter(|resource| resource.kind == ResourceKind::Database)
        .collect();
    if databases.len() > 1 {
        return Err(EngineError::Invalid(
            "A deployment supports one derived DATABASE_URL database.",
        ));
    }
    if let Some(database) = databases.first()
        && (!providers.can(database.provider, holds_databases)
            || !database.depends_on.is_empty()
            || plan.resources.iter().any(|resource| {
                resource.kind != ResourceKind::Database
                    && !resource.depends_on.contains(&database.name)
            }))
    {
        return Err(EngineError::Invalid(
            "Every runtime resource must depend on the planned database.",
        ));
    }
    if let Some(migration) = &plan.prisma_migration
        && (!migration.valid() || databases.len() != 1)
    {
        return Err(EngineError::Invalid(
            "Prisma migrations require a valid schema and one planned database.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::deploys::planning::*;

    /// Two made-up providers: a static and cron one, then one that runs containers,
    /// with a third that holds databases. Planning goes by what they can do.
    fn providers() -> Providers {
        Providers::new(vec![
            (
                Provider::Vercel,
                Capabilities {
                    plan_deploy: true,
                    cron: true,
                    ..Capabilities::NONE
                },
            ),
            (
                Provider::Render,
                Capabilities {
                    plan_deploy: true,
                    containers: true,
                    ..Capabilities::NONE
                },
            ),
            (
                Provider::Supabase,
                Capabilities {
                    project_database: true,
                    ..Capabilities::NONE
                },
            ),
        ])
    }

    fn accounts() -> Vec<Account> {
        [Provider::Vercel, Provider::Render, Provider::Supabase]
            .into_iter()
            .map(|provider| Account {
                last_discovery: None,
                id: provider.slug().into(),
                provider,
                label: provider.slug().into(),
                external_id: "sam".into(),
                scopes: vec![],
                status: AccountStatus::Connected,
                connected_at: "2026-11-01T10:00:00Z".into(),
                credential: None,
                in_1password: false,
            })
            .collect()
    }

    fn detected(shape: &str) -> Detection {
        Detection {
            schedules: None,
            framework: "fixture".into(),
            shape: shape.into(),
            package_manager: None,
            build_command: vec![],
            output_directory: "dist".into(),
            env_names: vec![],
            generated_secret_names: vec![],
            prisma_migration: None,
            postgres: false,
            warnings: vec![],
        }
    }

    fn planned(detection: Detection, pinned: Option<Provider>) -> Result<Plan, EngineError> {
        plan(
            Source::Folder {
                path: "/sam/shop".into(),
            },
            "shop",
            detection,
            &accounts(),
            &providers(),
            pinned,
            None,
        )
    }

    #[test]
    fn the_first_provider_that_can_run_it_is_the_default() {
        let site = planned(detected("static"), None).unwrap();
        assert_eq!(site.resources[0].provider, Provider::Vercel);
        assert_eq!(site.resources[0].kind, ResourceKind::Web);
        let service = planned(detected("service"), None).unwrap();
        assert_eq!(service.resources[0].provider, Provider::Render);
        assert_eq!(service.resources[0].kind, ResourceKind::Service);
    }

    #[test]
    fn a_provider_is_held_to_what_it_can_do() {
        assert!(planned(detected("static"), Some(Provider::Supabase)).is_err());
        assert!(planned(detected("service"), Some(Provider::Vercel)).is_err());
        let mut cron = detected("job");
        cron.schedules = Some(vec!["0 2 * * *".into()]);
        assert!(planned(cron.clone(), Some(Provider::Render)).is_err());
        let job = planned(cron, None).unwrap();
        assert!(release_order(&job.resources, &providers()).is_ok());
        let mut moved = job.resources.clone();
        moved[0].provider = Provider::Render;
        assert!(release_order(&moved, &providers()).is_err());
    }

    #[test]
    fn postgres_goes_to_the_provider_that_holds_databases() {
        let mut needs = detected("static");
        needs.postgres = true;
        let plan = planned(needs, None).unwrap();
        assert_eq!(plan.resources[0].provider, Provider::Supabase);
        assert_eq!(plan.resources[0].kind, ResourceKind::Database);
        assert!(validate_database_plan(&plan, &providers()).is_ok());
        assert!(validate_database_plan(&plan, &Providers::default()).is_err());
    }

    #[test]
    fn providers_are_named_in_the_order_they_are_suggested() {
        assert_eq!(providers().names(deploys), "Vercel or Render");
        assert_eq!(providers().names(runs_cron), "Vercel");
        assert_eq!(Providers::default().names(deploys), "no provider");
    }
}
