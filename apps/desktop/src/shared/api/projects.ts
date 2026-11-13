/**
 * Projects, their resources and how the resources connect.
 *
 * A project is one product as the user thinks of it; it is made of resources that
 * live on one or more providers. Endpoints: `GET /projects`, `GET /projects/{id}`,
 * `GET /projects/{id}/map`, `GET /resources/{id}`, `GET /projects/{id}/logs`,
 * `GET|PUT /projects/{id}/secrets`, `GET /projects/{id}/domains`.
 */
import type { Page, ProviderId, Timestamp } from './core';
import type { Actor } from './deploys';

/** Resource statuses. Project status is the worst status among its resources. */
export type Status =
  | 'live'
  | 'sleeping'
  | 'idle'
  | 'failing'
  | 'paused'
  /** A deploy is building or starting at the provider. */
  | 'deploying'
  /** The provider has no deployment of it yet. */
  | 'not_deployed'
  /** Stopped or removed at the provider until the next deploy. */
  | 'stopped'
  /** Not checked yet, or the provider couldn't be reached last time. */
  | 'unknown';

/** `managed`: Hoista deploys and changes it. `watching`: read-only, imported from a provider. */
export type ProjectMode = 'managed' | 'watching';

/** What kind of thing a resource is. */
export type ResourceKind = 'web' | 'service' | 'job' | 'database' | 'bucket' | 'email' | 'domain';

/** One row of the Projects list. */
export interface ProjectSummary {
  id: string;
  name: string;
  /** Public address, or null for projects with nothing public (a cron worker). */
  url: string | null;
  status: Status;
  /** Days without requests, when `status` is `idle`. */
  idle_days: number | null;
  mode: ProjectMode;
  /** The Hoista environment it lives in; `env_default` for default. */
  environment_id: string;
  /** Imported projects only: where they came from. */
  imported: boolean;
  /** Display names of the providers and services the resources run on. */
  runs_on: string[];
  providers: ProviderId[];
  /** Null when the provider does not report request counts (Railway). */
  requests_7d: number | null;
  /** Seven daily request counts, oldest first. Null when unknown. */
  sparkline: number[] | null;
  /** Who started it: an agent by name, or you in Hoista. */
  last_deploy: { at: Timestamp; status: 'live' | 'failed'; by: Actor } | null;
  /** False for a project of only data (a database, a bucket): nothing in it deploys. */
  ships_code?: boolean;
}

/**
 * A Hoista environment: a separate space for projects, such as `default` or
 * `production`. Agents can be limited to some. A provider's own environments (such
 * as a preview environment) are a different thing.
 * `GET/POST /environments`, `PATCH/DELETE /environments/{id}`,
 * `PUT /projects/{id}/environment`.
 */
export interface Environment {
  id: string;
  name: string;
  created_at: Timestamp;
}

/** A row of `GET /projects`. */
export type ProjectListItem = ProjectSummary;

/** `GET /projects` */
export interface ProjectList extends Page<ProjectListItem> {
  totals: { projects: number; live: number; failing: number; providers: number };
}

/** Filters and sort for `GET /projects`. */
export interface ProjectListQuery {
  /** Only projects in this Hoista environment (its id). */
  environment?: string;
  status?: Status;
  provider?: ProviderId;
  sort?: 'last_deploy' | 'requests' | 'name';
  q?: string;
}

/** One resource in a project. */
export interface Resource {
  id: string;
  project_id: string;
  kind: ResourceKind;
  provider: ProviderId;
  /** Product name at the provider, for example `Workers`, `Postgres`, `R2`. */
  product: string;
  name: string;
  region: string | null;
  status: Status;
  /** The provider's own id, shown for support and copy-paste. */
  provider_ref: string;
  /** One short line, for example `0.4 GB of 3 GB` or `12,480 requests`. */
  usage: string | null;
  /** The public address this resource serves, when it serves one. */
  url?: string | null;
  /** The GitHub repository its provider deploys it from, `owner/name`, on the map. */
  repo?: string | null;
}

/** Headline numbers on the app overview, last 7 days. */
export interface ProjectStats {
  requests: number | null;
  error_rate: number | null;
  p99_ms: number | null;
  /** Seven daily request counts, oldest first. */
  requests_daily: number[] | null;
  /** Requests this week divided by requests last week, for example 3.1. */
  requests_change: number | null;
  /** Failed requests in the last 24 hours. Null when the provider doesn't say. */
  errors_24h: number | null;
  /** The most common failure in that time, for example 413 on POST /api/upload. */
  top_error: { status: number; method: string; path: string } | null;
  /** A question worth asking your agent about the app right now, if any. */
  agent_question: string | null;
}

/** `GET /projects/{id}` */
export interface ProjectDetail {
  /** `unlinked`: imported or created without a source to deploy from. */
  project: ProjectSummary & {
    source: {
      kind: 'folder' | 'github' | 'unlinked';
      ref: string;
      /** A folder that is a clone of a GitHub repository: read from its git remote. */
      git?: {
        repository: string;
        branch: string | null;
        commit: string | null;
        temporary: boolean;
      };
    };
  };
  resources: Resource[];
  stats: ProjectStats;
}

/** A connection between two resources, for example web → database via `DATABASE_URL`. */
export interface Link {
  from: string;
  to: string;
  /** How they connect: an env var name or a binding. */
  via: string;
}

/** `GET /projects/{id}/map` */
export interface ProjectMap {
  nodes: Resource[];
  edges: Link[];
  /** Requests per minute along each edge, keyed `from>to`. */
  traffic: Record<string, number>;
}

/** A headline number on a resource's page, for example Storage · 212 MB · tables and indexes. */
export interface ResourceStat {
  label: string;
  value: string;
  note: string | null;
}

/** `GET /resources/{id}` */
export interface ResourceDetail {
  resource: Resource;
  /** One line under the title, for example `Neon · branch main · us-east-2 · Postgres 16`. */
  summary: string;
  /** What the status means for this kind of resource, for example `wakes on the next request`. */
  status_note: string | null;
  stats: ResourceStat[];
  used_by: { resource_id: string; name: string; provider: ProviderId; via: string }[];
  /** Recent provider events, newest first. */
  events: { at: Timestamp; title: string; tone: 'good' | 'bad' | 'warn' | 'neutral' }[];
  /** Labelled settings, read-only here; changes happen in the provider console. */
  settings: { label: string; value: string }[];
  /** Provider console link for this resource. */
  console_url: string;
}

/** One log line. */
export interface LogLine {
  at: Timestamp;
  resource_id: string;
  resource_name: string;
  /** Where the line came from: the app itself, a request it served, a provider's events, or a build. */
  source: 'project' | 'request' | 'provider' | 'build';
  level: 'info' | 'warn' | 'error';
  /** What happened, for example `413 Payload Too Large · 11.8 MB` or `200 · 72ms`. */
  message: string;
  /** The request or event the line is about, for example `POST /api/upload` or `compute woke`. */
  what: string;
  /** What kind of resource it came from, for the Resource column. Null for builds. */
  resource_kind: ResourceKind | null;
  /** HTTP status for request lines. */
  status: number | null;
  /** The version that was serving, for project lines. */
  version: string | null;
  /** Extra labelled facts shown when the line is opened, such as the request body size. */
  facts: { label: string; value: string }[];
  /** For problems: what it means and what to ask your agent. */
  hint: string | null;
}

/** Filters for `GET /projects/{id}/logs`. */
export interface LogQuery {
  level?: LogLine['level'];
  source?: LogLine['source'];
  /** Minutes back from now. */
  since_minutes?: number;
  q?: string;
  /** One resource only, on that resource's page; every resource otherwise. */
  resource?: string;
}

/** `GET /projects/{id}/secrets`. Secret values are never returned. */
export interface Secret {
  name: string;
  resource_id: string;
  resource_name: string;
  set_at: Timestamp;
  /** Short fingerprint so a change is visible without showing the value. */
  fingerprint: string;
  /** Set by Hoista itself (for example `DATABASE_URL` from a link); the provider is named. */
  managed_by: ProviderId | null;
  /** Bundled into the site's code, so anyone can read it (`VITE_`, `NEXT_PUBLIC_`). */
  visible_to_browser: boolean;
  /** The code reads it but it has no value yet. */
  missing: boolean;
  /** Where the code reads it, such as `src/errors.ts`, when Hoista found that. */
  read_in: string | null;
  /** The start of the value, only for values that are not secret (bucket names, public keys). */
  preview: string | null;
  /**
   * The other service this one's value comes from, on providers that link them
   * (Railway's `${{Postgres.DATABASE_URL}}`). Null for a value of its own.
   */
  links_to?: string | null;
  /** One of the project's shared variables, which every service in it can use. */
  shared?: boolean;
}

/** `PUT /projects/{id}/secrets/{name}` body. The value is sent once and never read back. */
export interface SetSecret {
  value: string;
}

/** `GET /projects/{id}/activity` (proposed): the project's slice of the event log, newest first. */
export interface ActivityItem {
  id: string;
  at: Timestamp;
  actor: { kind: 'person' | 'agent' | 'system'; name: string };
  tone: 'good' | 'bad' | 'warn' | 'neutral';
  title: string;
  detail: string | null;
  /** Where the item links to inside the app, if anywhere. */
  link: 'logs' | 'deploys' | 'map' | null;
  /** Hoista keeping its records up to date, not something that happened to the app. */
  housekeeping?: boolean;
}

/**
 * `GET /projects/{id}/activity/{item}`: the full record behind one line, like an
 * audit entry. For an agent's change, the call it sent and how it ended.
 */
export interface ActivityRecord {
  /** Hoista's name for what happened, such as `provider.change` or `deploy.live`. */
  kind: string;
  /** Who did it, as the engine recorded them: `person`, `hoista` or an agent's id. */
  actor_id: string;
  /** The agent's tool, when an agent did it. */
  tool: string | null;
  /** The call as the agent sent it, pretty-printed. Secret values are never kept. */
  sent: string | null;
  /** How it ended in a word or two, when an agent did it. */
  status: string | null;
  /** What the provider or Hoista said about it, one line each; empty when nothing more is known. */
  happened: string[];
  approval_id: string | null;
}

/** `GET /projects/{id}/domains` (proposed; not yet in the backend spec). */
export interface Domain {
  name: string;
  /** On a watched project: the resource at the provider that serves it. */
  resource_id?: string;
  /** `default` is the address the provider gives every project; `custom` is the user's own. */
  kind: 'default' | 'custom';
  status: 'active' | 'pending_dns' | 'error';
  /** `primary` is the main address; `redirect` sends visitors to `redirects_to`. */
  role: 'primary' | 'redirect' | null;
  redirects_to: string | null;
  /** Days until the HTTPS certificate renews, for active custom domains. */
  ssl_renews_in_days: number | null;
  /** On a watched project: the port traffic goes to, when the provider says. */
  port?: number | null;
  /** On a watched project: the certificate in a few words (`Valid`, `Issuing`, `Failed: …`). */
  certificate?: string | null;
  /** On a watched project: every DNS record it uses, checked against what the DNS host serves. */
  dns?: { type: string; name: string; value: string; current: string | null; ok: boolean }[];
  /** On a watched project: the service it reaches. */
  reaches?: string;
  /** While `pending_dns`: what to change, and where. */
  setup: {
    /** Where the domain's DNS is managed, such as `sam.dev` or `Namecheap`. */
    dns_host: string | null;
    registrar: string | null;
    /** Records to add at the DNS host. Empty when nameservers change instead. */
    records: { type: string; name: string; value: string }[];
    nameservers: string[];
    help_url: string | null;
  } | null;
}

/** `DELETE /projects/{id}` body. */
export interface RemoveProject {
  /** Download a database backup before deleting. */
  backup: boolean;
}

/**
 * `POST /projects/{id}/source/preview` with `{ path }`: what Hoista found in a
 * folder before it's connected. Looking links, builds and deploys nothing.
 */
export interface SourcePreview {
  path: string;
  framework: string | null;
  /** Static, edge, service or supabase, when recognised. */
  shape: string | null;
  package_manager: string | null;
  build_command: string[];
  /** Variable names the code reads; never values. */
  env_names: string[];
  warnings: string[];
  /** Set when Hoista didn't recognise the project, saying why. */
  unrecognised: string | null;
}

/** How a provider builds and runs a resource, as far as it says. */
export interface ProviderConfig {
  repo: string | null;
  image: string | null;
  start_command: string | null;
  build_command: string | null;
  root_directory: string | null;
  cron_schedule: string | null;
  healthcheck_path: string | null;
  replicas: number | null;
  region: string | null;
  restart_policy: string | null;
}

/** What a resource's provider reports about it: `GET /resources/:id/provider`. */
export interface ProviderDetails {
  deployments: {
    id: string;
    state: string;
    /** The provider's own word, for example Railway's `SUCCESS` or `REMOVED`. */
    status?: string;
    created_at: string;
    commit: string | null;
    message: string | null;
    repo?: string | null;
    branch?: string | null;
  }[];
  variable_names: string[] | null;
  domains: { name: string; custom: boolean; state: string | null }[];
  references: { variable: string; service: string }[];
  sleeps: boolean | null;
  config: ProviderConfig | null;
  volumes: {
    id: string;
    name: string;
    mount_path: string | null;
    used_mb?: number | null;
    size_mb?: number | null;
  }[];
  tcp_proxies: { id: string; domain: string; proxy_port: number; application_port: number }[];
  /** For example `postgres.railway.internal`. */
  private_domain?: string | null;
}

export type TcpProxy = ProviderDetails['tcp_proxies'][number];
