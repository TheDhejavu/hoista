//! HTTP routes the desktop app uses for catalog workflows. Agent tools call
//! the engine directly, not through these.

use super::contracts::ErrorDto;
use crate::{
    Provider,
    application::{
        engine::{Actor, Engine},
        ports::catalog::{EngineError, Page},
    },
    domain::{
        Secret,
        deletion::{DeletionConfirmation, DeletionPreview},
        model::*,
    },
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State, rejection::JsonRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

const MAX_VALUE_BYTES: usize = 32 * 1024;
/// The largest backup a restore accepts.
const MAX_BACKUP_BYTES: usize = 8 * 1024 * 1024;

/// Creates the desktop-facing routes around the shared engine.
pub fn router(engine: Arc<Engine>) -> Router {
    Router::new()
        .route("/providers", get(providers))
        .route("/vault", get(vault))
        .route("/doctor/accounts", get(account_diagnostics))
        .route("/database-exports", get(database_archives))
        .route(
            "/backup",
            get(export)
                .post(restore)
                .layer(DefaultBodyLimit::max(MAX_BACKUP_BYTES)),
        )
        .route("/accounts", get(accounts).post(connect))
        .route("/accounts/lookup", post(account_choices))
        .route("/accounts/{id}", delete(disconnect))
        .route("/accounts/{id}/check", post(check_account))
        .route("/accounts/sign-in", post(start_sign_in))
        .route("/server", post(set_up_server).delete(forget_server))
        .route("/server/connect", post(connect_server))
        .route("/operations", get(operations))
        .route("/build-isolation", get(build_isolation))
        .route("/webhooks", get(webhooks).post(add_webhook))
        .route("/webhooks/{id}", delete(remove_webhook))
        .route("/webhooks/{id}/rotate", post(rotate_webhook))
        .route("/webhooks/{id}/deliveries", get(webhook_deliveries))
        .route("/operations/{id}/check", post(check_operation))
        .route("/accounts/sign-in/{id}/finish", post(finish_sign_in))
        .route("/accounts/sign-in/{id}/callback", post(relay_sign_in))
        .route("/projects", get(projects).post(create_project))
        .route("/projects/summaries", get(project_summaries))
        .route("/projects/totals", get(project_totals))
        .route("/projects/{id}", get(project_detail).delete(delete_project))
        .route("/projects/{id}/merge", post(merge_project))
        .route("/projects/{id}/resources/move", post(move_resources))
        .route("/projects/{id}/name", put(rename_project))
        .route("/projects/{id}/environment", put(move_project))
        .route("/environments", get(environments).post(create_environment))
        .route(
            "/environments/{id}",
            axum::routing::patch(rename_environment).delete(remove_environment),
        )
        .route("/resources/{id}/split", post(split_resource))
        .route(
            "/resources/{id}/stop-watching",
            post(stop_watching_resource),
        )
        .route("/resources/{id}/provider", get(provider_details))
        .route("/resources/{id}/metrics", get(provider_metrics))
        .route("/accounts/{id}/repositories", get(repositories))
        .route("/accounts/{id}/empty-projects", get(empty_projects))
        .route("/accounts/{id}/branches", get(branches))
        .route("/accounts/{id}/projects", post(new_project_at_provider))
        .route("/projects/{id}/repository", put(link_repository))
        .route("/bills", get(bills))
        .route("/billing", get(billing))
        .route("/events/{id}/detail", get(event_detail))
        .route("/resources/{id}/provider/actions", post(provider_action))
        .route("/resources/{id}/provider/upload", post(provider_upload))
        .route(
            "/resources/{id}/provider/operations",
            get(resource_operations),
        )
        .route(
            "/resources/{id}/provider/quick-actions",
            get(resource_quick_actions),
        )
        .route(
            "/resources/{id}/provider/operations/{name}",
            post(run_resource_operation),
        )
        .route(
            "/accounts/{id}/provider/operations",
            get(account_operations),
        )
        .route(
            "/accounts/{id}/provider/operations/{name}",
            post(run_account_operation),
        )
        .route("/projects/{id}/stop-watching", post(stop_watching))
        .route(
            "/resources/{id}/provider/deployments/{deployment}/logs",
            get(provider_deployment_logs),
        )
        .route(
            "/resources/{id}/provider/variables/{name}",
            put(set_provider_variable).delete(delete_provider_variable),
        )
        .route(
            "/resources/{id}/provider/variables",
            put(set_provider_variables),
        )
        .route(
            "/resources/{id}/provider/shared-variables/{name}",
            delete(delete_shared_variable),
        )
        .route(
            "/projects/{id}/deletion-preview",
            get(project_deletion_preview),
        )
        .route("/projects/{id}/mode", put(mode))
        .route("/projects/{id}/map", get(project_map))
        .route("/projects/{id}/map/layout", put(save_map_layout))
        .route("/projects/{id}/domains", get(domains).post(add_domain))
        .route("/domains/{id}", delete(delete_domain))
        .route("/domains/{id}/check", post(check_domain))
        .route("/projects/{id}/deploys", get(deploys).post(enqueue))
        .route("/deploys", get(recent_deploys))
        .route("/deploys/{id}", get(deploy))
        .route("/deployment-requests/{key}", get(deployment_request))
        .route("/deploys/{id}/cancel", post(cancel_deploy))
        .route("/deploys/{id}/rollback", post(rollback))
        .route("/projects/{id}/logs", get(logs))
        .route("/projects/{id}/logs/live", get(live_logs))
        .route("/projects/{id}/logs/history", get(log_history))
        .route("/projects/{id}/usage", get(usage))
        .route("/projects/{id}/secrets", get(secrets))
        .route("/projects/{id}/secrets/{name}", put(set_secret))
        .route(
            "/projects/{id}/resources/{resource_id}/secrets/{name}",
            delete(delete_secret),
        )
        .route(
            "/projects/{id}/environment/deploy",
            post(replace_environment),
        )
        .route(
            "/projects/{id}/environment/public",
            post(public_environment),
        )
        .route("/resources/{id}", get(resource).delete(delete_resource))
        .route("/resources/{id}/details", get(resource_detail))
        .route(
            "/resources/{id}/relationships/refresh",
            post(refresh_bindings),
        )
        .route("/resources/{id}/database", get(database_observation))
        .route(
            "/resources/{id}/database/operations",
            get(database_operations),
        )
        .route(
            "/resources/{id}/deletion-preview",
            get(resource_deletion_preview),
        )
        .route("/resources/{id}/export", post(export_database))
        .route("/resources/{id}/pause", post(pause_resource))
        .route("/resources/{id}/resume", post(resume_resource))
        .route("/plans", post(plan))
        .route("/plans/{id}", get(get_plan).put(edit_plan))
        .route("/import/candidates", get(candidates))
        .route("/resources/{id}/ignored", put(set_ignored))
        .route("/import/suggestions", get(import_suggestions))
        .route("/import/resolve", post(resolve_description))
        .route("/import/group-description", post(group_description))
        .route("/import/scan", post(discover))
        .route("/import", post(group))
        .route("/import/projects", post(import_project))
        .route("/projects/{id}/source", put(link_source))
        .route("/projects/{id}/source/preview", post(preview_source))
        .route("/links", post(link))
        .route("/agents", get(agents))
        .route("/agents/{id}", delete(revoke_agent))
        .route("/agents/stop-all", post(stop_all_agents))
        .route("/agents/{id}/scopes", put(agent_scopes))
        .route("/agents/{id}/grant", put(agent_grant))
        .route("/agents/connection-requests", get(connection_requests))
        .route(
            "/agents/connection-requests/{id}",
            post(answer_connection_request),
        )
        .route(
            "/agents/oauth-requests/{id}",
            get(oauth_request).post(answer_oauth_request),
        )
        .route("/agents/{id}/reconnect", post(reconnect_agent))
        .route("/approvals", get(approvals))
        .route("/approvals/{id}", get(read_approval).post(approval))
        .merge(super::approval_display::router())
        .route("/settings", get(settings).patch(update_settings))
        .route("/recovery/environment-key", get(environment_key_recovery))
        .route(
            "/recovery/environment-key/retry-setup",
            post(retry_environment_key_setup),
        )
        .route("/attention", get(attention))
        .layer(DefaultBodyLimit::max(crate::MAX_REQUEST_BYTES))
        .with_state(engine)
}

pub(crate) struct ApiFault(pub(crate) EngineError);
impl From<EngineError> for ApiFault {
    fn from(error: EngineError) -> Self {
        Self(error)
    }
}

impl IntoResponse for ApiFault {
    fn into_response(self) -> Response {
        let status = match self.0 {
            EngineError::NotFound => StatusCode::NOT_FOUND,
            EngineError::Invalid(_)
            | EngineError::Unsupported(_)
            | EngineError::MissingSecrets(_) => StatusCode::UNPROCESSABLE_ENTITY,
            EngineError::Conflict(_) | EngineError::Watching => StatusCode::CONFLICT,
            EngineError::ApprovalRequired => StatusCode::FORBIDDEN,
            EngineError::Store | EngineError::AgentsDisabled => StatusCode::SERVICE_UNAVAILABLE,
            EngineError::Provider { .. } => StatusCode::BAD_GATEWAY,
        };
        (status, Json(ErrorDto::from_engine(&self.0))).into_response()
    }
}

fn body<T>(input: Result<Json<T>, JsonRejection>) -> Result<T, ApiFault> {
    input.map(|Json(body)| body).map_err(|_| {
        EngineError::Invalid(
            "Send a valid JSON request matching the documented schema and size limit.",
        )
        .into()
    })
}

#[derive(Default, Deserialize)]
struct Cursor {
    cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountLookup {
    provider: Provider,
    token: Secret,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Connect {
    provider: Provider,
    label: String,
    external_id: String,
    token: Secret,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateProject {
    name: String,
    source: Source,
    mode: Mode,
    #[serde(default)]
    environment_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetMode {
    mode: Mode,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanInput {
    source: Source,
    name: String,
    pinned: Option<Provider>,
    /// The project the plan is for, when it has one: the plan then keeps the
    /// provider its code runs on and uses the project's own database.
    project_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scan {
    account_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    project_id: String,
    resource_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Merge {
    into: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveResources {
    resource_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Named {
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    allow: bool,
    confirmation: Option<DeletionConfirmation>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SecretInput {
    resource_id: String,
    value: Secret,
}

/// Where provider tokens are kept, as the vault in use says.
async fn vault(State(engine): State<Arc<Engine>>) -> Json<crate::domain::model::VaultInfo> {
    Json(engine.vault_info().clone())
}

/// Every provider the engine has an adapter for: its names, its pages for making
/// an account and a token, and what its adapter can do.
async fn providers(State(engine): State<Arc<Engine>>) -> Json<Value> {
    let items: Vec<Value> = Provider::ALL
        .into_iter()
        .filter_map(|provider| {
            let adapter = engine.adapter(provider).ok()?;
            Some(json!({
                "provider": provider,
                "name": provider.display_name(),
                "account_label": provider.account_label(),
                "links": {"signup": provider.signup_page(), "tokens": provider.token_page()},
                "capabilities": adapter.capabilities(),
            }))
        })
        .collect();
    Json(json!({"items": items, "next_cursor": null}))
}

async fn accounts(
    State(engine): State<Arc<Engine>>,
    Query(query): Query<Cursor>,
) -> Result<Json<Page<Account>>, ApiFault> {
    Ok(Json(
        engine.repository.accounts(query.cursor.as_deref()).await?,
    ))
}

async fn account_choices(
    State(engine): State<Arc<Engine>>,
    input: Result<Json<AccountLookup>, JsonRejection>,
) -> Result<Json<Vec<AccountChoice>>, ApiFault> {
    let input = body(input)?;
    Ok(Json(
        engine
            .accounts()
            .account_choices(input.provider, input.token)
            .await?,
    ))
}

async fn connect(
    State(engine): State<Arc<Engine>>,
    input: Result<Json<Connect>, JsonRejection>,
) -> Result<Json<Account>, ApiFault> {
    let input = body(input)?;
    Ok(Json(
        engine
            .accounts()
            .connect_account(input.provider, input.label, input.external_id, input.token)
            .await?,
    ))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WebhookInput {
    url: String,
    #[serde(default)]
    events: Vec<String>,
    #[serde(default)]
    project_id: Option<String>,
}

async fn webhooks(
    State(engine): State<Arc<Engine>>,
) -> Result<Json<Vec<crate::domain::model::Webhook>>, ApiFault> {
    Ok(Json(engine.webhooks().list().await?))
}

async fn add_webhook(
    State(engine): State<Arc<Engine>>,
    input: Result<Json<WebhookInput>, JsonRejection>,
) -> Result<Json<crate::application::engine::NewWebhook>, ApiFault> {
    let input = body(input)?;
    Ok(Json(
        engine
            .webhooks()
            .add_webhook(&Actor::Person, &input.url, input.events, input.project_id)
            .await?,
    ))
}

async fn remove_webhook(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiFault> {
    engine
        .webhooks()
        .remove_webhook(&Actor::Person, &id)
        .await?;
    Ok(Json(json!({"removed": true})))
}

async fn rotate_webhook(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<crate::application::engine::NewWebhook>, ApiFault> {
    Ok(Json(
        engine
            .webhooks()
            .rotate_webhook(&Actor::Person, &id)
            .await?,
    ))
}

async fn webhook_deliveries(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::domain::model::WebhookDelivery>>, ApiFault> {
    Ok(Json(engine.webhooks().webhook_deliveries(&id).await?))
}

/// How project build code is kept apart on this computer, for Settings → Safety.
async fn build_isolation(State(engine): State<Arc<Engine>>) -> Json<Value> {
    Json(match engine.settings().build_isolation() {
        Ok(how) => json!({ "isolated": true, "how": how, "why_not": null }),
        Err(why) => json!({ "isolated": false, "how": null, "why_not": why }),
    })
}

async fn operations(
    State(engine): State<Arc<Engine>>,
    Query(query): Query<Cursor>,
) -> Result<Json<Page<crate::domain::model::Operation>>, ApiFault> {
    Ok(Json(
        engine.observe().operations(query.cursor.as_deref()).await?,
    ))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckInput {
    /// What the person saw at the provider.
    happened: bool,
}

async fn check_operation(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<CheckInput>, JsonRejection>,
) -> Result<Json<crate::domain::model::Operation>, ApiFault> {
    let input = body(input)?;
    Ok(Json(
        engine
            .observe()
            .check_operation(&Actor::Person, &id, input.happened)
            .await?,
    ))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerInput {
    account_id: String,
    /// Build from this copy of the repository instead of the published image.
    #[serde(default)]
    repo: Option<String>,
    branch: Option<String>,
}

async fn set_up_server(
    State(engine): State<Arc<Engine>>,
    input: Result<Json<ServerInput>, JsonRejection>,
) -> Result<Json<crate::application::engine::ServerSetup>, ApiFault> {
    let input = body(input)?;
    Ok(Json(
        engine
            .accounts()
            .set_up_server(
                &Actor::Person,
                &input.account_id,
                input.repo.as_deref(),
                input.branch.as_deref(),
            )
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectServer {
    url: String,
}

async fn connect_server(
    State(engine): State<Arc<Engine>>,
    input: Result<Json<ConnectServer>, JsonRejection>,
) -> Result<Json<Settings>, ApiFault> {
    let input = body(input)?;
    Ok(Json(
        engine
            .accounts()
            .connect_server(&Actor::Person, &input.url)
            .await?,
    ))
}

async fn forget_server(State(engine): State<Arc<Engine>>) -> Result<Json<Settings>, ApiFault> {
    Ok(Json(engine.accounts().forget_server(&Actor::Person).await?))
}

async fn check_account(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<Account>, ApiFault> {
    Ok(Json(engine.accounts().check_account(&id).await?))
}

async fn disconnect(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiFault> {
    engine.accounts().disconnect_account(&id).await?;
    Ok(Json(json!({"removed":true})))
}

async fn projects(
    State(engine): State<Arc<Engine>>,
    Query(query): Query<Cursor>,
) -> Result<Json<Page<Project>>, ApiFault> {
    Ok(Json(
        engine.repository.projects(query.cursor.as_deref()).await?,
    ))
}

async fn project_summaries(
    State(engine): State<Arc<Engine>>,
    Query(query): Query<crate::application::engine::ProjectListQuery>,
) -> Result<Json<Page<crate::application::engine::ProjectOverview>>, ApiFault> {
    Ok(Json(engine.accounts().project_summaries(query).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InEnvironment {
    environment: Option<String>,
}

async fn project_totals(
    State(engine): State<Arc<Engine>>,
    Query(query): Query<InEnvironment>,
) -> Result<Json<crate::application::engine::ProjectTotals>, ApiFault> {
    Ok(Json(
        engine
            .accounts()
            .project_totals_in(query.environment.as_deref())
            .await?,
    ))
}

async fn create_project(
    State(engine): State<Arc<Engine>>,
    input: Result<Json<CreateProject>, JsonRejection>,
) -> Result<Json<Project>, ApiFault> {
    let input = body(input)?;
    Ok(Json(
        engine
            .projects()
            .create_project_in(
                &Actor::Person,
                input.environment_id.as_deref(),
                input.name,
                input.source,
                input.mode,
            )
            .await?,
    ))
}

async fn project_detail(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<crate::application::engine::ProjectOverview>, ApiFault> {
    Ok(Json(engine.projects().project_overview(&id).await?))
}

async fn mode(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<SetMode>, JsonRejection>,
) -> Result<Json<Project>, ApiFault> {
    Ok(Json(
        engine.projects().set_mode(&id, body(input)?.mode).await?,
    ))
}

async fn project_map(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<crate::application::engine::ProjectMap>, ApiFault> {
    Ok(Json(
        engine.projects().project_map(&Actor::Person, &id).await?,
    ))
}

async fn database_observation(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<crate::domain::database_observation::DatabaseObservation>, ApiFault> {
    Ok(Json(
        engine
            .projects()
            .database_observation(&Actor::Person, &id)
            .await?,
    ))
}

async fn resource_detail(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<crate::application::engine::ResourceDetail>, ApiFault> {
    Ok(Json(
        engine
            .projects()
            .resource_detail(&Actor::Person, &id)
            .await?,
    ))
}

async fn resource(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<Resource>, ApiFault> {
    Ok(Json(
        engine
            .repository
            .resource(&id)
            .await?
            .ok_or(EngineError::NotFound)?,
    ))
}

async fn pause_resource(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<Resource>, ApiFault> {
    Ok(Json(
        engine
            .deploys()
            .control_resource(&Actor::Person, &id, false)
            .await?,
    ))
}

async fn resume_resource(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<Resource>, ApiFault> {
    Ok(Json(
        engine
            .deploys()
            .control_resource(&Actor::Person, &id, true)
            .await?,
    ))
}

async fn deploys(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    Query(query): Query<Cursor>,
) -> Result<Json<Page<Deploy>>, ApiFault> {
    Ok(Json(
        engine
            .repository
            .deploys(&id, query.cursor.as_deref())
            .await?,
    ))
}

async fn recent_deploys(State(engine): State<Arc<Engine>>) -> Result<Json<Page<Deploy>>, ApiFault> {
    Ok(Json(Page {
        items: engine.repository.recent_deploys().await?,
        next_cursor: None,
    }))
}

async fn deploy(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<Deploy>, ApiFault> {
    Ok(Json(
        engine
            .repository
            .deploy(&id)
            .await?
            .ok_or(EngineError::NotFound)?,
    ))
}

async fn secrets(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    Query(query): Query<Cursor>,
) -> Result<Json<Value>, ApiFault> {
    engine
        .repository
        .project(&id)
        .await?
        .ok_or(EngineError::NotFound)?;
    let page = engine
        .repository
        .secrets_page(&id, query.cursor.as_deref())
        .await?;
    Ok(Json(json!({
        "items": page.items.into_iter().map(EnvironmentVariable::from).collect::<Vec<_>>(),
        "next_cursor": page.next_cursor,
    })))
}

async fn replace_environment(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<crate::domain::environment::EnvironmentDeployRequest>, JsonRejection>,
) -> Result<Json<Deploy>, ApiFault> {
    let input = body(input)?;
    Ok(Json(
        engine
            .environments()
            .replace_environment(
                &Actor::Person,
                &id,
                &input.plan_id,
                input
                    .changes
                    .into_iter()
                    .map(|change| (change.name, change.value))
                    .collect(),
                DeployOptions {
                    request_key: input.request_key,
                    expected_plan_revision: input.expected_plan_revision,
                    title: input.title,
                    channel: Some(input.channel.unwrap_or(DeployChannel::Desktop)),
                },
            )
            .await?,
    ))
}

async fn delete_secret(
    State(engine): State<Arc<Engine>>,
    Path((id, resource_id, name)): Path<(String, String, String)>,
) -> Result<Json<Value>, ApiFault> {
    engine
        .environments()
        .delete_secret(&Actor::Person, &id, &resource_id, &name)
        .await?;
    Ok(Json(json!({
        "resource_id": resource_id,
        "name": name,
        "provider_secret_removed": true,
        "source_changed": false,
        "browser_assets_rebuilt": false,
    })))
}

async fn set_secret(
    State(engine): State<Arc<Engine>>,
    Path((id, name)): Path<(String, String)>,
    input: Result<Json<SecretInput>, JsonRejection>,
) -> Result<Json<Value>, ApiFault> {
    let input = body(input)?;
    let value = engine
        .environments()
        .set_secret(&Actor::Person, &id, &input.resource_id, &name, input.value)
        .await?;
    Ok(Json(json!(EnvironmentVariable::from(value))))
}

async fn plan(
    State(engine): State<Arc<Engine>>,
    input: Result<Json<PlanInput>, JsonRejection>,
) -> Result<Json<Plan>, ApiFault> {
    let input = body(input)?;
    let plan = match input.project_id {
        Some(project) => {
            engine
                .projects()
                .plan_in_project(
                    &Actor::Person,
                    &project,
                    input.source,
                    &input.name,
                    input.pinned,
                )
                .await?
        }
        None => {
            engine
                .deploys()
                .plan(
                    &Actor::Person,
                    input.source,
                    &input.name,
                    input.pinned,
                    None,
                )
                .await?
        }
    };
    Ok(Json(plan))
}

async fn get_plan(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<Plan>, ApiFault> {
    Ok(Json(
        engine
            .repository
            .plan(&id)
            .await?
            .ok_or(EngineError::NotFound)?,
    ))
}

async fn import_suggestions(
    State(engine): State<Arc<Engine>>,
    Query(query): Query<Cursor>,
) -> Result<Json<Page<crate::application::engine::ImportProposal>>, ApiFault> {
    Ok(Json(
        engine
            .accounts()
            .import_suggestions(&Actor::Person, query.cursor.as_deref())
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidateQuery {
    cursor: Option<String>,
    #[serde(default)]
    ignored: bool,
}

async fn candidates(
    State(engine): State<Arc<Engine>>,
    Query(query): Query<CandidateQuery>,
) -> Result<Json<Page<Resource>>, ApiFault> {
    Ok(Json(
        engine
            .accounts()
            .import_candidates(&Actor::Person, query.ignored, query.cursor.as_deref())
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IgnoredInput {
    ignored: bool,
}

async fn set_ignored(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<IgnoredInput>, JsonRejection>,
) -> Result<Json<Value>, ApiFault> {
    let ignored = body(input)?.ignored;
    engine
        .accounts()
        .set_resource_ignored(&Actor::Person, &id, ignored)
        .await?;
    Ok(Json(
        json!({"id":id,"ignored":ignored,"cloud_changed":false}),
    ))
}

async fn discover(
    State(engine): State<Arc<Engine>>,
    input: Result<Json<Scan>, JsonRejection>,
) -> Result<Json<DiscoveryResult>, ApiFault> {
    Ok(Json(
        engine
            .accounts()
            .discover(&Actor::Person, &body(input)?.account_id)
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportProject {
    name: String,
    resource_ids: Vec<String>,
    source: Option<Source>,
    mode: Option<Mode>,
    #[serde(default)]
    environment_id: Option<String>,
}

async fn import_project(
    State(engine): State<Arc<Engine>>,
    input: Result<Json<ImportProject>, JsonRejection>,
) -> Result<Json<crate::application::engine::ImportedProject>, ApiFault> {
    let input = body(input)?;
    Ok(Json(
        engine
            .projects()
            .import_project_in(
                &Actor::Person,
                input.environment_id.as_deref(),
                input.name,
                input.source,
                input.mode.unwrap_or(Mode::Watching),
                &input.resource_ids,
            )
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkSource {
    source: Source,
}

async fn link_source(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<LinkSource>, JsonRejection>,
) -> Result<Json<Project>, ApiFault> {
    Ok(Json(
        engine
            .projects()
            .link_source(&Actor::Person, &id, body(input)?.source)
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewSource {
    path: String,
}

async fn preview_source(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<PreviewSource>, JsonRejection>,
) -> Result<Json<crate::application::deploys::planning::SourcePreview>, ApiFault> {
    Ok(Json(
        engine
            .projects()
            .preview_source(&Actor::Person, &id, body(input)?.path)
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VariableValue {
    value: String,
}

async fn set_provider_variable(
    State(engine): State<Arc<Engine>>,
    Path((id, name)): Path<(String, String)>,
    input: Result<Json<VariableValue>, JsonRejection>,
) -> Result<StatusCode, ApiFault> {
    let input = body(input)?;
    if input.value.is_empty() || input.value.len() > MAX_VALUE_BYTES {
        return Err(EngineError::Invalid("Variable values are 1 byte to 32 KiB.").into());
    }
    engine
        .environments()
        .set_provider_variable(
            &Actor::Person,
            &id,
            &name,
            Some(crate::domain::Secret::new(input.value)),
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VariablesInput {
    variables: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    shared: bool,
    #[serde(default = "yes")]
    redeploy: bool,
}

fn yes() -> bool {
    true
}

async fn set_provider_variables(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<VariablesInput>, JsonRejection>,
) -> Result<StatusCode, ApiFault> {
    let input = body(input)?;
    let values = input
        .variables
        .into_iter()
        .map(|(name, value)| (name, crate::domain::Secret::new(value)))
        .collect();
    engine
        .environments()
        .set_provider_variables(&Actor::Person, &id, values, input.shared, input.redeploy)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_shared_variable(
    State(engine): State<Arc<Engine>>,
    Path((id, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiFault> {
    engine
        .environments()
        .delete_shared_variable(&Actor::Person, &id, &name)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_provider_variable(
    State(engine): State<Arc<Engine>>,
    Path((id, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiFault> {
    engine
        .environments()
        .set_provider_variable(&Actor::Person, &id, &name, None)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn provider_deployment_logs(
    State(engine): State<Arc<Engine>>,
    Path((id, deployment)): Path<(String, String)>,
) -> Result<Json<Vec<crate::domain::logs::LogEntry>>, ApiFault> {
    Ok(Json(
        engine
            .observe()
            .provider_deployment_logs(&Actor::Person, &id, &deployment)
            .await?,
    ))
}

/// `POST /resources/{id}/provider/actions` body.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields, tag = "action", rename_all = "snake_case")]
pub(crate) enum ProviderActionInput {
    /// Build and start the current version again.
    Redeploy,
    /// Restart the running deployment without rebuilding.
    Restart,
    /// Serve an earlier deployment again.
    Rollback {
        /// The provider's id of that deployment, from the resource's provider details.
        deployment_id: String,
    },
    /// Deploy the newest commit of the linked repository.
    DeployLatest,
    /// Let the provider put it to sleep when idle, or keep it awake.
    Sleep {
        /// True to let it sleep.
        on: bool,
    },
    /// Stop the running deployment.
    Stop,
    /// Deploy one commit of the linked repository.
    DeployCommit {
        /// The full commit hash.
        sha: String,
    },
    /// Change how the provider builds and runs it; only the fields given change.
    Configure {
        /// The settings to change.
        config: crate::application::ports::providers::ServiceConfig,
    },
    /// Deploy from a GitHub repository from now on.
    ConnectRepo {
        /// `owner/name`.
        repo: String,
        /// The branch; the repository's default when absent.
        branch: Option<String>,
    },
    /// Add an address: your own domain, or a free provider address when absent.
    AddDomain {
        /// Such as `api.example.com`.
        domain: Option<String>,
        /// The port the service listens on, when not the default.
        port: Option<u16>,
    },
    /// Delete the resource at its provider (a provider project, service, Worker, bucket,
    /// sending domain or zone), with everything in it.
    RemoveService {
        /// The resource's name, typed by the person to confirm.
        confirmation_name: String,
    },
    /// Remove an address.
    RemoveDomain {
        /// The address as listed in provider details.
        domain: String,
    },
    /// Point an address at another port.
    UpdateDomain {
        /// The address as listed in provider details.
        domain: String,
        /// The port the service listens on.
        port: u16,
    },
    /// Give the provider's own address a new name.
    RenameDomain {
        /// The address as listed in provider details.
        domain: String,
        /// The new address.
        to: String,
    },
    /// Ask the provider to issue a custom domain's certificate again.
    RetryCertificate {
        /// The custom domain.
        domain: String,
    },
    /// Open a public TCP endpoint to a port.
    AddTcpProxy {
        /// The port the service listens on.
        port: u16,
    },
    /// Close a public TCP endpoint.
    RemoveTcpProxy {
        /// The endpoint's id from provider details.
        proxy_id: String,
    },
    /// Rename the private network address.
    RenamePrivateHost {
        /// The short name, without the provider's suffix.
        name: String,
    },
}

impl ProviderActionInput {
    pub(crate) fn action(&self) -> crate::application::ports::providers::ProviderAction<'_> {
        use crate::application::ports::providers::ProviderAction;
        match self {
            Self::Redeploy => ProviderAction::Redeploy,
            Self::Restart => ProviderAction::Restart,
            Self::Rollback { deployment_id } => ProviderAction::Rollback { deployment_id },
            Self::DeployLatest => ProviderAction::DeployLatest,
            Self::Sleep { on } => ProviderAction::Sleep { on: *on },
            Self::Stop => ProviderAction::Stop,
            Self::DeployCommit { sha } => ProviderAction::DeployCommit { sha },
            Self::Configure { config } => ProviderAction::Configure(config),
            Self::ConnectRepo { repo, branch } => ProviderAction::ConnectRepo {
                repo,
                branch: branch.as_deref(),
            },
            Self::AddDomain { domain, port } => ProviderAction::AddDomain {
                domain: domain.as_deref(),
                port: *port,
            },
            Self::RemoveService { .. } => ProviderAction::RemoveService,
            Self::RemoveDomain { domain } => ProviderAction::RemoveDomain { domain },
            Self::UpdateDomain { domain, port } => ProviderAction::UpdateDomain {
                domain,
                port: *port,
            },
            Self::RetryCertificate { domain } => ProviderAction::RetryCertificate { domain },
            Self::RenameDomain { domain, to } => ProviderAction::RenameDomain { domain, to },
            Self::AddTcpProxy { port } => ProviderAction::AddTcpProxy { port: *port },
            Self::RemoveTcpProxy { proxy_id } => ProviderAction::RemoveTcpProxy { proxy_id },
            Self::RenamePrivateHost { name } => ProviderAction::RenamePrivateHost { name },
        }
    }
}

/// `POST /resources/{id}/provider/upload` body.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadInput {
    subfolder: Option<String>,
    message: Option<String>,
}

/// `POST …/provider/operations/{name}` body: the operation's arguments.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct OperationInput {
    #[serde(default)]
    args: serde_json::Value,
}

async fn resource_operations(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::application::ports::providers::ProviderOperationInfo>>, ApiFault> {
    use crate::application::engine::OperationTarget;
    Ok(Json(
        engine
            .deploys()
            .provider_operations(&Actor::Person, &OperationTarget::Resource(id))
            .await?,
    ))
}

async fn resource_quick_actions(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::application::ports::providers::ProviderQuickAction>>, ApiFault> {
    use crate::application::engine::OperationTarget;
    Ok(Json(
        engine
            .deploys()
            .provider_quick_actions(&Actor::Person, &OperationTarget::Resource(id))
            .await?,
    ))
}

async fn account_operations(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::application::ports::providers::ProviderOperationInfo>>, ApiFault> {
    use crate::application::engine::OperationTarget;
    Ok(Json(
        engine
            .deploys()
            .provider_operations(&Actor::Person, &OperationTarget::Account(id))
            .await?,
    ))
}

async fn run_resource_operation(
    State(engine): State<Arc<Engine>>,
    Path((id, name)): Path<(String, String)>,
    input: Result<Json<OperationInput>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiFault> {
    use crate::application::engine::OperationTarget;
    let input = body(input)?;
    Ok(Json(
        engine
            .deploys()
            .provider_operation(
                &Actor::Person,
                OperationTarget::Resource(id),
                &name,
                input.args,
                None,
            )
            .await?,
    ))
}

async fn run_account_operation(
    State(engine): State<Arc<Engine>>,
    Path((id, name)): Path<(String, String)>,
    input: Result<Json<OperationInput>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiFault> {
    use crate::application::engine::OperationTarget;
    let input = body(input)?;
    Ok(Json(
        engine
            .deploys()
            .provider_operation(
                &Actor::Person,
                OperationTarget::Account(id),
                &name,
                input.args,
                None,
            )
            .await?,
    ))
}

async fn provider_upload(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<UploadInput>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiFault> {
    let input = body(input)?;
    let summary = engine
        .deploys()
        .provider_upload(
            &Actor::Person,
            &id,
            input.subfolder.as_deref(),
            input.message.as_deref(),
        )
        .await?;
    Ok(Json(serde_json::json!({ "summary": summary })))
}

async fn stop_watching(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiFault> {
    engine.projects().stop_watching(&Actor::Person, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn stop_watching_resource(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiFault> {
    engine
        .projects()
        .stop_watching_resource(&Actor::Person, &id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn provider_action(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<ProviderActionInput>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiFault> {
    let input = body(input)?;
    // Deleting needs the resource's exact name, as the person read it.
    if let ProviderActionInput::RemoveService { confirmation_name } = &input {
        let resource = engine
            .repository
            .resource(&id)
            .await?
            .ok_or(EngineError::NotFound)?;
        if confirmation_name != &resource.name {
            return Err(
                EngineError::Invalid("Type the resource's name exactly to delete it.").into(),
            );
        }
    }
    let summary = engine
        .deploys()
        .provider_action(&Actor::Person, &id, input.action())
        .await?;
    Ok(Json(serde_json::json!({ "summary": summary })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignIn {
    provider: crate::Provider,
    /// The app listens on the person's computer and forwards the answer.
    #[serde(default)]
    relay: bool,
}

async fn start_sign_in(
    State(engine): State<Arc<Engine>>,
    input: Result<Json<SignIn>, JsonRejection>,
) -> Result<Json<crate::application::engine::SignInStart>, ApiFault> {
    let input = body(input)?;
    Ok(Json(
        engine
            .accounts()
            .start_sign_in(&Actor::Person, input.provider, input.relay)
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignInCallback {
    state: String,
    code: Option<String>,
}

async fn relay_sign_in(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<SignInCallback>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiFault> {
    let input = body(input)?;
    engine
        .accounts()
        .relay_callback(&Actor::Person, &id, &input.state, input.code.as_deref())?;
    Ok(Json(serde_json::json!({ "relayed": true })))
}

async fn finish_sign_in(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<Account>, ApiFault> {
    Ok(Json(
        engine
            .accounts()
            .finish_sign_in(&Actor::Person, &id)
            .await?,
    ))
}

/// What an agent's call sent and how it ended, with secret values removed
/// when it was recorded. Null for events that keep no detail.
async fn event_detail(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiFault> {
    let id = id.parse::<i64>().map_err(|_| EngineError::NotFound)?;
    let detail = engine.repository.event_detail(id).await?;
    Ok(Json(serde_json::json!({
        "detail": detail.and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
    })))
}

async fn bills(
    State(engine): State<Arc<Engine>>,
) -> Result<Json<Vec<crate::application::engine::AccountBill>>, ApiFault> {
    Ok(Json(engine.accounts().bills(&Actor::Person).await?))
}

async fn billing(
    State(engine): State<Arc<Engine>>,
) -> Result<Json<Vec<crate::application::engine::AccountBilling>>, ApiFault> {
    Ok(Json(engine.accounts().billing(&Actor::Person).await?))
}

async fn provider_details(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<crate::application::ports::providers::ProviderDetails>, ApiFault> {
    Ok(Json(
        engine
            .projects()
            .provider_details(&Actor::Person, &id)
            .await?,
    ))
}

async fn empty_projects(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::application::ports::providers::ProviderEmptyProject>>, ApiFault> {
    Ok(Json(
        engine
            .projects()
            .empty_provider_projects(&Actor::Person, &id)
            .await?,
    ))
}

async fn repositories(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::application::ports::providers::ProviderRepository>>, ApiFault> {
    Ok(Json(
        engine.accounts().repositories(&Actor::Person, &id).await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BranchesQuery {
    repository: String,
}

async fn branches(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    Query(query): Query<BranchesQuery>,
) -> Result<Json<Vec<String>>, ApiFault> {
    Ok(Json(
        engine
            .accounts()
            .branches(&Actor::Person, &id, &query.repository)
            .await?,
    ))
}

/// `POST /accounts/{id}/projects` body: a new project at the account's provider, built
/// from a GitHub repository.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewProjectAtProvider {
    /// The project's name, lowercase letters, digits and hyphens.
    name: String,
    /// `owner/name` on GitHub.
    repository: String,
    /// The branch; the repository's default when absent.
    branch: Option<String>,
}

async fn new_project_at_provider(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<NewProjectAtProvider>, JsonRejection>,
) -> Result<Json<Value>, ApiFault> {
    let input = body(input)?;
    let (imported, summary) = engine
        .deploys()
        .new_project_at_provider(
            &Actor::Person,
            &id,
            input.name,
            &input.repository,
            input.branch.as_deref(),
        )
        .await?;
    Ok(Json(
        json!({ "project": imported.project, "resources": imported.resources, "summary": summary }),
    ))
}

/// `PUT /projects/{id}/repository` body.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LinkRepository {
    /// The code account (GitHub) that reaches it.
    account_id: String,
    /// `owner/name`.
    repository: String,
    /// The branch; the repository's default when absent.
    branch: Option<String>,
}

async fn link_repository(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<LinkRepository>, JsonRejection>,
) -> Result<Json<Project>, ApiFault> {
    let input = body(input)?;
    Ok(Json(
        engine
            .projects()
            .link_repository(
                &Actor::Person,
                &id,
                &input.account_id,
                &input.repository,
                input.branch.as_deref(),
            )
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MetricsQuery {
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    step: u32,
}

async fn provider_metrics(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    Query(query): Query<MetricsQuery>,
) -> Result<Json<crate::application::ports::providers::ProviderMetrics>, ApiFault> {
    Ok(Json(
        engine
            .observe()
            .provider_metrics(&Actor::Person, &id, query.from, query.to, query.step)
            .await?,
    ))
}

async fn rename_project(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<Named>, JsonRejection>,
) -> Result<Json<Project>, ApiFault> {
    let input = body(input)?;
    Ok(Json(
        engine
            .projects()
            .rename_project(&Actor::Person, &id, &input.name)
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveProject {
    environment_id: String,
}

async fn move_project(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<MoveProject>, JsonRejection>,
) -> Result<Json<Project>, ApiFault> {
    let input = body(input)?;
    Ok(Json(
        engine
            .projects()
            .move_project(&Actor::Person, &id, &input.environment_id)
            .await?,
    ))
}

async fn environments(
    State(engine): State<Arc<Engine>>,
) -> Result<Json<Vec<Environment>>, ApiFault> {
    Ok(Json(engine.environments().list(&Actor::Person).await?))
}

async fn create_environment(
    State(engine): State<Arc<Engine>>,
    input: Result<Json<Named>, JsonRejection>,
) -> Result<Json<Environment>, ApiFault> {
    let input = body(input)?;
    Ok(Json(
        engine
            .environments()
            .create_environment(&Actor::Person, &input.name)
            .await?,
    ))
}

async fn rename_environment(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<Named>, JsonRejection>,
) -> Result<Json<Environment>, ApiFault> {
    let input = body(input)?;
    Ok(Json(
        engine
            .environments()
            .rename_environment(&Actor::Person, &id, &input.name)
            .await?,
    ))
}

async fn remove_environment(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<axum::http::StatusCode, ApiFault> {
    engine
        .environments()
        .remove_environment(&Actor::Person, &id)
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn split_resource(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<Named>, JsonRejection>,
) -> Result<Json<Project>, ApiFault> {
    let input = body(input)?;
    Ok(Json(
        engine
            .projects()
            .split_resource(&Actor::Person, &id, &input.name)
            .await?,
    ))
}

/// Moves resources from wherever they are into this project.
async fn move_resources(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<MoveResources>, JsonRejection>,
) -> Result<Json<Value>, ApiFault> {
    let input = body(input)?;
    let (moved, removed) = engine
        .projects()
        .move_resources(&Actor::Person, &input.resource_ids, &id)
        .await?;
    Ok(Json(json!({"moved": moved, "removed_projects": removed})))
}

async fn merge_project(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<Merge>, JsonRejection>,
) -> Result<Json<Project>, ApiFault> {
    let input = body(input)?;
    Ok(Json(
        engine
            .projects()
            .merge_projects(&Actor::Person, &id, &input.into)
            .await?,
    ))
}

async fn group(
    State(engine): State<Arc<Engine>>,
    input: Result<Json<Group>, JsonRejection>,
) -> Result<Json<Value>, ApiFault> {
    let input = body(input)?;
    Ok(Json(json!({
        "items": engine.projects().group(&Actor::Person,&input.project_id,&input.resource_ids).await?,
        "next_cursor": null,
    })))
}

async fn link(
    State(engine): State<Arc<Engine>>,
    input: Result<Json<Link>, JsonRejection>,
) -> Result<Json<Link>, ApiFault> {
    let mut value = body(input)?;
    value.id = uuid::Uuid::new_v4().to_string();
    value.derived = false;
    value.missing = false;
    engine.projects().save_link(&value).await?;
    Ok(Json(value))
}

async fn agents(
    State(engine): State<Arc<Engine>>,
    Query(query): Query<Cursor>,
) -> Result<Json<Page<Agent>>, ApiFault> {
    Ok(Json(
        engine.repository.agents(query.cursor.as_deref()).await?,
    ))
}

async fn stop_all_agents(State(engine): State<Arc<Engine>>) -> Result<Json<Value>, ApiFault> {
    let revoked = engine.agents().stop_all_agents(&Actor::Person).await?;
    Ok(Json(json!({ "revoked": revoked, "agents_enabled": false })))
}

async fn revoke_agent(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiFault> {
    engine.agents().revoke_agent(&Actor::Person, &id).await?;
    Ok(Json(json!({"revoked":true})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentScopes {
    scopes: Vec<String>,
}

async fn reconnect_agent(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<Agent>, ApiFault> {
    Ok(Json(
        engine.agents().reconnect_agent(&Actor::Person, &id).await?,
    ))
}

async fn agent_scopes(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<AgentScopes>, JsonRejection>,
) -> Result<Json<Agent>, ApiFault> {
    Ok(Json(
        engine
            .agents()
            .set_agent_scopes(&Actor::Person, &id, body(input)?.scopes)
            .await?,
    ))
}

async fn connection_requests(
    State(engine): State<Arc<Engine>>,
) -> Result<Json<serde_json::Value>, ApiFault> {
    Ok(Json(
        serde_json::json!({ "items": engine.accounts().enrollment_requests().await? }),
    ))
}

/// The person's answer to a coding tool asking to connect.
#[derive(serde::Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct ConnectionAnswer {
    /// Allow or deny.
    pub allow: bool,
    /// What it may do when allowed; defaults to read only.
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
    /// Which projects and environments it may touch; defaults to every project.
    #[serde(default)]
    pub grant: Option<crate::domain::model::AgentGrantInput>,
    /// Keep the earlier connections from the same tool on this computer;
    /// by default the new one replaces them.
    #[serde(default)]
    pub keep_earlier: bool,
}

async fn answer_connection_request(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<ConnectionAnswer>, JsonRejection>,
) -> Result<Json<crate::application::engine::EnrollmentRequest>, ApiFault> {
    let answer = body(input)?;
    Ok(Json(
        engine
            .accounts()
            .decide_enrollment(
                &Actor::Person,
                &id,
                answer.allow,
                answer.scopes.unwrap_or_else(|| vec!["apps:read".into()]),
                answer.grant,
                !answer.keep_earlier,
            )
            .await?,
    ))
}

/// A web agent waiting to sign in, for answering it in Hoista itself.
async fn oauth_request(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiFault> {
    let request = engine
        .accounts()
        .oauth_request(&id)
        .await?
        .ok_or(EngineError::NotFound)?;
    Ok(Json(serde_json::json!({
        "id": request.id,
        "name": request.name,
        "code": request.code,
        "expires_at": request.expires_at,
        // Where the browser goes back to: `127.0.0.1` for a tool on this computer.
        "returns_to": request.returns_to,
    })))
}

/// The person's answer to a web agent signing in. Allowing asks for Touch ID
/// or the password on this computer first.
async fn answer_oauth_request(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<ConnectionAnswer>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiFault> {
    let answer = body(input)?;
    let request = engine
        .accounts()
        .decide_oauth(
            &Actor::Person,
            &id,
            answer.allow,
            answer.scopes.unwrap_or_else(|| vec!["apps:read".into()]),
            answer.grant.unwrap_or_default(),
        )
        .await?;
    Ok(Json(
        serde_json::json!({ "id": request.id, "name": request.name }),
    ))
}

async fn agent_grant(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<crate::domain::model::AgentGrantInput>, JsonRejection>,
) -> Result<Json<Agent>, ApiFault> {
    Ok(Json(
        engine
            .agents()
            .set_agent_grant(&Actor::Person, &id, body(input)?)
            .await?,
    ))
}

async fn approvals(
    State(engine): State<Arc<Engine>>,
    Query(query): Query<Cursor>,
) -> Result<Json<Page<Approval>>, ApiFault> {
    Ok(Json(
        engine.repository.approvals(query.cursor.as_deref()).await?,
    ))
}

async fn read_approval(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<Approval>, ApiFault> {
    Ok(Json(
        engine
            .repository
            .approval(&id)
            .await?
            .ok_or(EngineError::NotFound)?,
    ))
}

async fn approval(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<Decision>, JsonRejection>,
) -> Result<Json<Value>, ApiFault> {
    let input = body(input)?;
    engine
        .projects()
        .decide_approval(&id, input.allow, input.confirmation.as_ref())
        .await?;
    Ok(Json(json!({"resolved":true})))
}

async fn settings(State(engine): State<Arc<Engine>>) -> Result<Json<Settings>, ApiFault> {
    Ok(Json(engine.repository.settings().await?))
}

async fn update_settings(
    State(engine): State<Arc<Engine>>,
    input: Result<Json<SettingsPatch>, JsonRejection>,
) -> Result<Json<Settings>, ApiFault> {
    Ok(Json(
        engine
            .settings()
            .update_settings(&Actor::Person, body(input)?)
            .await?,
    ))
}

async fn attention(
    State(engine): State<Arc<Engine>>,
    Query(query): Query<crate::application::engine::AttentionQuery>,
) -> Result<Json<crate::application::engine::AttentionFeed>, ApiFault> {
    Ok(Json(engine.projects().attention(query).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueueInput {
    request_key: Option<String>,
    expected_plan_revision: Option<i64>,
    plan_id: String,
    title: Option<String>,
    channel: Option<DeployChannel>,
}

async fn enqueue(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<QueueInput>, JsonRejection>,
) -> Result<Json<Value>, ApiFault> {
    let input = body(input)?;
    let deploy = engine
        .deploys()
        .enqueue_deploy_with_options(
            &Actor::Person,
            &id,
            &input.plan_id,
            DeployOptions {
                request_key: input.request_key,
                expected_plan_revision: input.expected_plan_revision,
                title: input.title,
                channel: Some(input.channel.unwrap_or(DeployChannel::Desktop)),
            },
        )
        .await?;
    Ok(Json(json!({"id":deploy.id,"status":deploy.status})))
}

async fn delete_project(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<DeletionConfirmation>, JsonRejection>,
) -> Result<Json<Value>, ApiFault> {
    let archives = engine
        .deploys()
        .delete_project(&Actor::Person, &id, None, Some(&body(input)?))
        .await?;
    Ok(Json(json!({"removed":true,"database_archives":archives})))
}

async fn delete_resource(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<DeletionConfirmation>, JsonRejection>,
) -> Result<Json<Value>, ApiFault> {
    let archives = engine
        .projects()
        .delete_resource(&Actor::Person, &id, None, Some(&body(input)?))
        .await?;
    Ok(Json(json!({"removed":true,"database_archives":archives})))
}

async fn rollback(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Option<Json<DeployOptions>>, JsonRejection>,
) -> Result<Json<Deploy>, ApiFault> {
    let mut options = input
        .map_err(|_| EngineError::Invalid("Send a valid rollback request."))?
        .map(|Json(value)| value)
        .unwrap_or_default();
    options.channel = Some(options.channel.unwrap_or(DeployChannel::Desktop));
    Ok(Json(
        engine
            .deploys()
            .enqueue_rollback_with_options(&Actor::Person, &id, options)
            .await?,
    ))
}

async fn edit_plan(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<Plan>, JsonRejection>,
) -> Result<Json<Plan>, ApiFault> {
    let input = body(input)?;
    if input.id != id {
        return Err(EngineError::Invalid("Plan id must match the URL.").into());
    }
    Ok(Json(
        engine.deploys().edit_plan(&Actor::Person, input).await?,
    ))
}

#[derive(Deserialize)]
struct LogQuery {
    resource: String,
    since: String,
    q: Option<String>,
}

async fn logs(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    Query(query): Query<LogQuery>,
) -> Result<Json<Value>, ApiFault> {
    Ok(Json(json!(
        engine
            .observe()
            .logs(
                &Actor::Person,
                &id,
                &query.resource,
                &query.since,
                query.q.as_deref().unwrap_or("")
            )
            .await?
    )))
}

#[derive(Deserialize)]
struct HistoryQuery {
    resource: String,
    #[serde(flatten)]
    query: crate::domain::logs::LogHistoryQuery,
}

async fn log_history(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<crate::domain::logs::LogHistory>, ApiFault> {
    Ok(Json(
        engine
            .observe()
            .log_history(&Actor::Person, &id, &query.resource, &query.query)
            .await?,
    ))
}

#[derive(Deserialize)]
struct LiveLogQuery {
    resource: String,
    q: Option<String>,
    level: Option<crate::domain::logs::LogLevel>,
}

async fn live_logs(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    Query(query): Query<LiveLogQuery>,
) -> Result<Response, ApiFault> {
    use crate::application::engine::LogFrame;
    use axum::response::sse::{Event, KeepAlive, Sse};
    let mut subscription = engine
        .observe()
        .live_logs(
            &Actor::Person,
            &id,
            &query.resource,
            query.q.as_deref().unwrap_or(""),
            query.level,
        )
        .await?;
    let stream = async_stream::stream! {
        while let Some(frame) = subscription.recv().await {
            let name = match &frame { LogFrame::Ready { .. } => "ready", LogFrame::Log { .. } => "log", LogFrame::End { .. } => "end" };
            // All frame fields are strings, booleans or serializable log records.
            if let Ok(event) = Event::default().event(name).json_data(&frame) {
                yield Ok::<_, std::convert::Infallible>(event);
            }
        }
    };
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

#[derive(Deserialize)]
struct CostQuery {
    since: String,
    cursor: Option<String>,
}

async fn usage(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    Query(query): Query<CostQuery>,
) -> Result<Json<Value>, ApiFault> {
    engine
        .repository
        .project(&id)
        .await?
        .ok_or(EngineError::NotFound)?;
    chrono::NaiveDate::parse_from_str(&query.since, "%Y-%m-%d")
        .map_err(|_| EngineError::Invalid("Use a YYYY-MM-DD date."))?;
    Ok(Json(json!(
        engine
            .repository
            .usage_page(&id, &query.since, query.cursor.as_deref())
            .await?
    )))
}

async fn export(
    State(engine): State<Arc<Engine>>,
) -> Result<Json<crate::domain::backup::Backup>, ApiFault> {
    Ok(Json(engine.data().export().await?))
}

async fn restore(
    State(engine): State<Arc<Engine>>,
    input: Result<Json<crate::domain::backup::Backup>, JsonRejection>,
) -> Result<Json<Value>, ApiFault> {
    engine.data().restore(body(input)?).await?;
    Ok(Json(json!({"restored":true,"mode":"watching"})))
}

async fn cancel_deploy(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiFault> {
    engine.deploys().cancel_deploy(&Actor::Person, &id).await?;
    Ok(Json(json!({
        "cancel_requested": true,
        "message": "Stopping at the next safe boundary; an in-flight provider operation may still complete.",
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DomainInput {
    resource_id: String,
    hostname: String,
    zone_name: String,
    #[serde(default)]
    create_zone: bool,
}

async fn domains(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    Query(query): Query<Cursor>,
) -> Result<Json<Page<crate::domain::domains::CustomDomain>>, ApiFault> {
    Ok(Json(
        engine
            .repository
            .custom_domains(Some(&id), query.cursor.as_deref())
            .await?,
    ))
}

async fn add_domain(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<DomainInput>, JsonRejection>,
) -> Result<Json<crate::domain::domains::CustomDomain>, ApiFault> {
    let input = body(input)?;
    Ok(Json(
        engine
            .environments()
            .add_domain(
                &Actor::Person,
                &id,
                &input.resource_id,
                &input.hostname,
                &input.zone_name,
                input.create_zone,
            )
            .await?,
    ))
}

async fn check_domain(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<crate::domain::domains::CustomDomain>, ApiFault> {
    Ok(Json(engine.environments().refresh_domain(&id).await?))
}

async fn delete_domain(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiFault> {
    engine
        .environments()
        .delete_domain(&Actor::Person, &id, None)
        .await?;
    Ok(Json(json!({"removed":true})))
}

async fn project_deletion_preview(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<DeletionPreview>, ApiFault> {
    Ok(Json(
        engine
            .projects()
            .preview_deletion(&Actor::Person, GatedAction::DeleteProject, &id)
            .await?,
    ))
}

async fn resource_deletion_preview(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<DeletionPreview>, ApiFault> {
    Ok(Json(
        engine
            .projects()
            .preview_deletion(&Actor::Person, GatedAction::DeleteResource, &id)
            .await?,
    ))
}

async fn export_database(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::domain::database_export::DatabaseArchive>>, ApiFault> {
    Ok(Json(
        engine.data().export_database(&Actor::Person, &id).await?,
    ))
}

async fn database_archives(
    State(engine): State<Arc<Engine>>,
    Query(query): Query<Cursor>,
) -> Result<Json<Page<crate::domain::database_export::DatabaseArchive>>, ApiFault> {
    Ok(Json(
        engine
            .data()
            .database_archives(&Actor::Person, query.cursor.as_deref())
            .await?,
    ))
}

async fn account_diagnostics(
    State(engine): State<Arc<Engine>>,
    Query(query): Query<Cursor>,
) -> Result<Json<Page<crate::application::engine::AccountDiagnostic>>, ApiFault> {
    Ok(Json(
        engine
            .observe()
            .diagnose_accounts(&Actor::Person, query.cursor.as_deref())
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestNamespace {
    actor: Option<String>,
}

async fn deployment_request(
    State(engine): State<Arc<Engine>>,
    Path(key): Path<String>,
    query: Result<Query<RequestNamespace>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<DeploymentRequest>, ApiFault> {
    let Query(query) =
        query.map_err(|_| EngineError::Invalid("Use a valid actor namespace query."))?;
    Ok(Json(
        engine
            .deploys()
            .deployment_request(&Actor::Person, &key, query.actor.as_deref())
            .await?,
    ))
}

async fn environment_key_recovery(
    State(engine): State<Arc<Engine>>,
) -> Result<Json<crate::application::environments::admission_keys::EnvironmentKeyRecovery>, ApiFault>
{
    Ok(Json(
        engine
            .accounts()
            .environment_key_recovery(&Actor::Person)
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeySetupReview {
    review_token: String,
}

async fn retry_environment_key_setup(
    State(engine): State<Arc<Engine>>,
    input: Result<Json<KeySetupReview>, JsonRejection>,
) -> Result<Json<crate::application::environments::admission_keys::EnvironmentKeyRecovery>, ApiFault>
{
    let input = body(input)?;
    Ok(Json(
        engine
            .accounts()
            .retry_environment_key_setup(&Actor::Person, &input.review_token)
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DescriptionInput {
    description: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DescriptionGroupInput {
    description: String,
    review_token: String,
    project_id: String,
    resource_ids: Vec<String>,
}

async fn resolve_description(
    State(engine): State<Arc<Engine>>,
    input: Result<Json<DescriptionInput>, JsonRejection>,
) -> Result<Json<crate::application::projects::description::DescriptionResolution>, ApiFault> {
    Ok(Json(
        engine
            .projects()
            .resolve_description(&Actor::Person, &body(input)?.description)
            .await?,
    ))
}

async fn group_description(
    State(engine): State<Arc<Engine>>,
    input: Result<Json<DescriptionGroupInput>, JsonRejection>,
) -> Result<Json<Vec<Resource>>, ApiFault> {
    let input = body(input)?;
    Ok(Json(
        engine
            .projects()
            .group_description(
                &Actor::Person,
                &input.description,
                &input.review_token,
                &input.project_id,
                &input.resource_ids,
            )
            .await?,
    ))
}

async fn save_map_layout(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<crate::domain::map_layout::MapLayout>, JsonRejection>,
) -> Result<Json<crate::domain::map_layout::MapLayout>, ApiFault> {
    Ok(Json(
        engine
            .projects()
            .save_map_layout(&Actor::Person, &id, body(input)?)
            .await?,
    ))
}

async fn database_operations(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    Query(query): Query<Cursor>,
) -> Result<Json<crate::domain::database_observation::DatabaseOperationHistory>, ApiFault> {
    Ok(Json(
        engine
            .projects()
            .database_operations(&Actor::Person, &id, query.cursor.as_deref())
            .await?,
    ))
}

async fn refresh_bindings(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<crate::application::engine::BindingRefresh>, ApiFault> {
    Ok(Json(
        engine
            .projects()
            .refresh_bindings(&Actor::Person, &id)
            .await?,
    ))
}

async fn public_environment(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
    input: Result<Json<crate::domain::environment::PublicEnvironmentRequest>, JsonRejection>,
) -> Result<impl IntoResponse, ApiFault> {
    let values = engine
        .environments()
        .public_environment(&Actor::Person, &id, body(input)?)
        .await?;
    Ok((
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(values),
    ))
}
