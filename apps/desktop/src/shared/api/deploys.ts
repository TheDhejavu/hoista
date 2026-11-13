/**
 * Deploys, roll backs and import.
 *
 * Endpoints: `GET|POST /projects/{id}/deploys`, `GET /deploys/{id}`,
 * `POST /deploys/{id}/rollback`, `GET /deploys` (the latest deploys across
 * all projects), `GET /deploys/{id}/log`, `GET /deploys/{id}/summary`, and
 * `GET /import/found`, `POST /import/projects`, `POST /import` for bringing
 * projects in.
 */
import type { ProviderId, Timestamp } from './core';
import type { ResourceKind } from './projects';

/** Deploy statuses, newest first everywhere. */
/** `cancelled` means someone stopped it before it went live; nothing on the live site changed. */
export type DeployStatus = 'queued' | 'running' | 'live' | 'failed' | 'rolled_back' | 'cancelled';

/** Pipeline steps, in order. */
export type StepName =
  'detect' | 'plan' | 'secrets' | 'leak_check' | 'build' | 'release' | 'verify';

/** One step of a deploy. */
export interface DeployStep {
  name: StepName;
  status: 'pending' | 'running' | 'done' | 'failed' | 'skipped';
  /** One plain line, for example `Vite project, 214 kB` or `Missing RESEND_API_KEY`. */
  detail: string | null;
  duration_ms: number | null;
}

/** Who started a deploy. */
export interface Actor {
  /** `system` is a provider deploying on its own (a Railway deploy from a push), named by the provider. */
  kind: 'person' | 'agent' | 'system';
  name: string;
}

/** Why a deploy failed and what to do about it. */
export interface DeployFailure {
  /** The pipeline step that was running, or null when it failed before any step started. */
  step: StepName | null;
  message: string;
  fix: string | null;
  /** Secrets the app needs that are not set, so the UI can ask for them. */
  missing_secrets?: string[];
}

/** One deploy of one project. */
export interface Deploy {
  id: string;
  project_id: string;
  /** `v1`, `v2`… */
  version: string;
  source: {
    kind: 'folder' | 'commit';
    ref: string;
    message: string | null;
    /** The repository the commit is in, `owner/name`, when known. */
    repo?: string;
    /** The commit's full id, when known. */
    commit?: string;
  };
  started_by: Actor;
  status: DeployStatus;
  steps: DeployStep[];
  url: string | null;
  created_at: Timestamp;
  duration_ms: number | null;
  /** True for the version serving traffic now. */
  current: boolean;
  /** For a deploy made at the provider: the provider's own status word, such as `NEEDS_APPROVAL`. */
  provider_status?: string;
  /** For a deploy made at the provider: whether the provider says it can go back to it. */
  can_roll_back?: boolean;
  /** For a deploy made at the provider: which provider made it. */
  provider?: ProviderId;
  /** For a deploy made at the provider: the resource it deployed. */
  resource_id?: string;
  /** For a deploy made at the provider: its page in the provider's dashboard. */
  provider_page?: string;
  /** When failed: what went wrong and what to do, in plain English. */
  failure: DeployFailure | null;
}

/** What a plan is tuned for. */
export type PlanGoal = 'fewest_accounts' | 'fastest';

/** Where the code for a deploy comes from. */
export type DeploySource =
  { kind: 'folder'; path: string } | { kind: 'github'; repo: string; branch: string };

/** One place a planned resource could run. */
export interface ProviderOption {
  /** For example `fly`, `cloudflare-cron`, `fly-address`. */
  id: string;
  /** For example `Fly.io` or `Same as project server (Fly.io)`. */
  label: string;
  provider: ProviderId | null;
  recommended: boolean;
  connected: boolean;
  /** Null when this option can be picked; otherwise why not, for example `needs Node`. */
  unavailable_reason: string | null;
  /** Why this provider suits the resource, shown while it is picked. */
  reason: string;
}

/** One resource a deploy would create or update, with where it could run. */
export interface PlannedResource {
  /** Stable within the plan, for example `project`, `database`, `job`, `address`. */
  id: string;
  kind: ResourceKind;
  /** For example `Project server`. */
  label: string;
  /** For example `Next.js 15 · Node APIs`. */
  detail: string;
  options: ProviderOption[];
  /** The id of the option Hoista picked. */
  chosen: string;
}

/** `POST /deploys/plan`: what Hoista found and what it would do. Nothing is deployed yet. */
export interface DeployPlan {
  /** The project this becomes. Existing projects keep their id. */
  project_id: string;
  project_name: string;
  source: DeploySource;
  /** One line, for example `built with Cursor · last change 12 min ago`. */
  summary: string;
  detected: { label: string; value: string }[];
  goal: PlanGoal;
  resources: PlannedResource[];
  /** Secrets the user must provide, by name. */
  secrets_needed: string[];
  /** Secrets Hoista sets itself, for example `DATABASE_URL from Neon`. */
  secrets_handled: string[];
  /** Providers the chosen providers need that are not connected yet. */
  missing_accounts: ProviderId[];
}

/** `POST /deploys/plan` body. */
export interface PlanRequest {
  source: DeploySource;
  goal: PlanGoal;
}

/** `POST /projects/{id}/deploys` body. The engine creates the project on its first deploy. */
export interface StartDeploy {
  source: DeploySource;
  /** Resource id to provider option id. Left out on a redeploy to keep the current providers. */
  choices?: Record<string, string>;
  /** Secret values, sent once to the provider and never stored by Hoista. */
  secrets?: Record<string, string>;
}

/** `POST /projects/{id}/deploys` result. */
export interface StartedDeploy {
  deploy_id: string;
}

/** One line of build output. */
export interface DeployLogLine {
  /** Milliseconds since the deploy started. */
  offset_ms: number;
  level: 'info' | 'ok' | 'warn' | 'error';
  text: string;
}

/** `GET /deploys/{id}/summary`: where the deploy is going. */
export interface DeploySummary {
  /** Display names of the providers, for example `Fly.io`, `Neon`. */
  runs_on: string[];
  targets: { label: string; value: string }[];
}

/** A resource found on a connected provider that is not part of any project yet. */
export interface FoundResource {
  id: string;
  provider: ProviderId;
  kind: ResourceKind;
  name: string;
  url: string | null;
  /** A project already holding resources from the same provider project, to add this to. */
  join: { project_id: string; project_name: string } | null;
  /** The connected account it was found in, such as a Railway workspace. */
  account: { id: string; label: string };
  /** The provider's grouping by name, such as a Railway project; null when none. */
  group: string | null;
}

/** Two watched projects from the same provider project, offered to combine into one. */
export interface FoundCombine {
  from: { id: string; name: string };
  into: { id: string; name: string };
  /** The provider both are on. */
  provider: ProviderId;
  /** For example `Same Railway project`. */
  reason: string;
}

/** Resources the engine thinks belong together, and why. Never applied by itself. */
export interface FoundGroup {
  id: string;
  label: string;
  resource_ids: string[];
  /** For example `Same name: shop`. */
  reason: string;
  /** True when the resources also fit another suggestion; review before adding. */
  ambiguous: boolean;
}

/** `GET /import/found`: what connecting your providers found, ready to add as projects. */
export interface FoundOnProviders {
  groups: FoundGroup[];
  resources: FoundResource[];
  combine: FoundCombine[];
}

/** A project at the provider with nothing in it yet. `GET /accounts/{id}/empty-projects`. */
export interface EmptyProviderProject {
  id: string;
  name: string;
  /** Its page in the provider's dashboard. */
  url: string | null;
}

/** One resource inside a provider project, and the app that holds it, if any. */
export interface ProviderProjectResource {
  id: string;
  name: string;
  kind: ResourceKind;
  url: string | null;
  /** The project it is in; null while it sits under Found on your providers. */
  project_id: string | null;
}

/**
 * A provider's own project (a Railway project) inside one connected account (a
 * workspace), with everything in it. `GET /provider-projects?provider=railway`.
 */
export interface ProviderProject {
  /** Account id and project name; a resource's id when the provider has no projects. */
  key: string;
  provider: ProviderId;
  account: { id: string; label: string };
  name: string;
  /** The project holding this project's resources, when one does. */
  project: { id: string; name: string } | null;
  resources: ProviderProjectResource[];
}

/**
 * `POST /import/projects` body. Imported projects start in watching mode: Hoista
 * reads them and changes nothing at the provider.
 */
export interface ImportProjectRequest {
  name: string;
  resource_ids: string[];
  mode: 'watching';
  /** The Hoista environment it goes into; default when left out. */
  environment_id?: string;
}
