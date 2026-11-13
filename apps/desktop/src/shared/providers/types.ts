/**
 * What the app knows about one provider, in one place. Every provider is one file in
 * this folder; nothing else in the app names a provider. What a provider can do
 * comes from the engine (`GET /providers`), never from here.
 */
import type { CatalogueEntry, ConnectGuide, ServiceId } from '../api';

/** An icon as one SVG path on a 24×24 grid, in the brand's colour. */
export interface Logo {
  colour: string;
  path: string;
  /** For marks drawn with holes, such as Runpod's cube. */
  fillRule?: 'evenodd';
}

/** Where a provider sits on the Accounts screen. */
export type ProviderGroup = 'hosting' | 'gpu' | 'databases' | 'email' | 'code';

/** What the provider's own pages look like, for the step-by-step guides. */
export interface ProviderPages {
  /** The provider's brand colour, for its main buttons in the drawings. */
  colour: string;
  signupOptions: string[];
  /** The sidebar item the token page sits under, and the page's title. */
  tokensNav: string;
  tokensTitle: string;
  createLabel: string;
  fields: { label: string; value: string }[];
  /** What each numbered marker on the token page means. */
  tokenSteps: string[];
  usedFor: string;
  plan: string;
  /** Something that catches people out on this provider, shown beside the steps. */
  headsUp?: string;
}

/** Something the provider reports, shown as a table or facts on its tab. */
export interface ProviderView {
  op: string;
  title: string;
  about?: string;
  args?: Record<string, unknown>;
  /** Show only this field of the answer, such as the list inside it. */
  field?: string;
  /** What an empty answer says, such as `No changes waiting.` */
  empty?: string;
  /** What each row offers in its ⋮ menu, such as Rename or Delete for an environment. */
  rowActions?: ProviderRowAction[];
}

/** Something you can change on the provider's tab, as a button that opens its form. */
export interface ProviderAction {
  op: string;
  label: string;
  /** Arguments already chosen, such as `{ action: 'restart' }` for one pod action. */
  args?: Record<string, unknown>;
  /** A read whose answer fills the form with what's set now, such as the current limits. */
  current?: string;
}

/**
 * An action on one row of a view: the row's own value goes into `arg`
 * (`id` by default from the row's `id`), and its name is what a delete
 * asks you to type.
 */
export interface ProviderRowAction {
  op: string;
  label: string;
  /** The operation's argument that takes the row: `id`, or `input.id` inside an input. */
  arg?: string;
  /** The row field it comes from; `id` by default. */
  from?: string;
}

/** One section of the provider's tab, built from its operation catalog. */
export interface ProviderSection {
  id: string;
  label: string;
  about?: string;
  views: ProviderView[];
  actions: ProviderAction[];
  /** A section drawn by hand rather than from views. */
  special?: 'volumes';
  /**
   * Which resources it's for, by how the provider's reference starts
   * (`worker:` for a Cloudflare Worker); every resource when unset.
   */
  refs?: string[];
  /** Where the rest of it lives in Hoista. */
  link?: { tab: string; label: string };
}

/** How the provider names and measures what it runs, in its own words. */
export interface ProviderWords {
  /** The provider's product for a kind of resource, like `Workers` or `Service`. */
  product: (kind: string, reference?: string) => string;
  /**
   * What the provider calls one resource of this kind, singular, such as
   * `Worker` or `App`; the general kind (`Web service`) when it has no word.
   */
  term?: (kind: string, reference: string) => string | undefined;
  /** The compute and storage labels on a resource page. */
  compute: string;
  storage: string;
  /** Compute is measured in vCPU time. */
  vcpu: boolean;
  /** What a redeploy does there, in a few words. */
  redeploy?: string;
}

/** How the provider's raw field names and values read to people, on its tab. */
export interface FieldWords {
  /** Raw field name → its label, like `numReplicas` → `Replicas`. */
  labels: Record<string, string>;
  /** Raw values → words, like `ON_FAILURE` → `On failure`. */
  values: Record<string, string>;
  /** Units after a field's value, like `memoryGB` → `GB`. */
  units: Record<string, string>;
}

/** How a fact on a resource's page is written. */
export type FactFormat =
  'text' | 'code' | 'bytes' | 'gb' | 'mb' | 'ago' | 'bool' | 'seconds' | 'cu' | 'join';

/** One fact: what it's called, where it is in the provider's answer, and how to write it. */
export interface ProviderFact {
  label: string;
  /** Dot path into the answer, such as `project.region_id`. */
  path: string;
  as?: FactFormat;
}

/**
 * One section on a resource's page, read from one of the provider's own read
 * operations: facts about it (`rows`), or a table of something it holds
 * (`list` with `columns`). Sections the provider doesn't answer are left out.
 */
export interface ProviderFacts {
  title: string;
  /** A read in the provider's operation catalog, aimed at the resource. */
  op: string;
  args?: Record<string, unknown>;
  /** Resource kinds it's for; every kind when unset. */
  on?: string[];
  /** Provider ids it's for, by prefix, such as Cloudflare's `worker:`. */
  refs?: string[];
  rows?: ProviderFact[];
  /** The array to show as a table: a dot path, or '' for the answer itself. */
  list?: string;
  columns?: ProviderFact[];
  /** At most this many rows. */
  limit?: number;
  /** What happened to it lately: shown as Recent events. */
  events?: boolean;
}

export interface ProviderEntry {
  id: ServiceId;
  name: string;
  logo: Logo;
  /** Hoista can read existing projects from it and bring them in. */
  canImport: boolean;
  /** Its row in the catalogue. */
  catalogue: Pick<CatalogueEntry, 'description' | 'category'>;
  /** How to connect it. Its pages for an account and a token come from the engine. */
  connect: Omit<ConnectGuide, 'service' | 'name' | 'signup_url' | 'token_url'>;
  /** How to name the account when a token can't list its accounts. */
  accountId?: { label: string; hint: string };
  /** Its tile on Accounts, when it's offered there. */
  tile?: { for: string; group: ProviderGroup };
  /** Its step-by-step guide. */
  pages?: ProviderPages;
  words?: ProviderWords;
  /** The provider's console for a resource, or its dashboard. */
  console?: (reference: string, account: string) => string;
  /**
   * The resource's console from one of its deployments' pages, for a provider
   * whose console addresses need names Hoista doesn't keep (Vercel's team).
   */
  consoleFrom?: (deploymentPage: string) => string | null;
  /** What a resource's page shows about it, from the provider's own reads. */
  facts?: ProviderFacts[];
  /** The provider's own dashboard for a connected account, from its name there. */
  dashboard?: (identity: string) => string;
  /** The provider's billing page for the signed-in account. */
  billing?: string;
  /**
   * The provider's own words for a token's access, as its token settings name
   * them (GitHub's "Read and write", Cloudflare's "Edit"). Unset: Read, Read
   * and write, Read, write and delete.
   */
  access?: { read: string; write: string; delete: string };
  /** What an account's Overview shows about it, from the provider's account-wide reads. */
  accountFacts?: ProviderFacts[];
  /** The provider project a resource belongs to, where the provider has projects. */
  project?: (reference: string) => string | null;
  /** What the provider calls a connected account, as its own UI does: Workspace, Team, Account, Organization. */
  accountWord?: string;
  /** What the provider calls the group its resources live in (a Railway "project"); unset when it has none. */
  groupWord?: string;
  /** What the provider calls the things Hoista finds on an account; `Projects` when unset. */
  projectsWord?: string;
  /**
   * The provider project an address points at, when it is one of this provider's:
   * how a variable on another provider (`DATABASE_URL`) shows it uses this one.
   */
  owns?: (address: { host: string; user: string | null }) => string | null;
  /**
   * Variable names that show a service uses this provider even when they hold
   * no address, such as Resend's `RESEND_API_KEY`.
   */
  usedVia?: RegExp;
  /** The provider can open a public TCP endpoint in front of a resource (Railway's TCP proxy). */
  tcpEndpoints?: boolean;
  /** Hoista can change the GitHub repository a watched resource deploys from (Railway's Connect Repo). */
  connectRepo?: boolean;
  /** What Hoista can change about a watched app's domains at this provider; unset, none. */
  domainActions?: DomainActions;
  /** Its tab on a watched app, from its operation catalog. */
  sections?: ProviderSection[];
  /** How its fields read on that tab. */
  fieldWords?: FieldWords;
  /** For the Welcome drawings: what a saved token looks like. */
  sampleToken?: string;
  /** Logs are kept for its database services too. */
  databaseLogs?: boolean;
  /**
   * How its variables work: whether a project can share them across
   * resources, and whether setting one redeploys the resource.
   */
  variables?: {
    shared?: boolean;
    redeploys?: boolean;
    /** The kinds of resource that hold them, when only some do (Supabase: the database). */
    on?: string[];
  };
  /** Other kinds of resource it keeps logs for, such as storage buckets. */
  logKinds?: string[];
  /**
   * Its own request counts over time, for the Metrics tab: the read that
   * gives them, the interval for each range, and each series by field.
   */
  usageSeries?: {
    op: string;
    field: string;
    intervals: Partial<Record<'1h' | '24h' | '7d' | '30d', string>>;
    series: Record<string, string>;
    /** The read that says how full the disk is: `metrics.fs_used_bytes` of `fs_size_bytes`. */
    disk?: string;
  };
  /**
   * Which resources the provider reports numbers for, by reference prefix
   * (Cloudflare: Workers and zones, not D1 or KV). All of them when left out.
   */
  metricsRefs?: string[];
  /**
   * Metrics the engine reads from the provider as charts (GET
   * /resources/:id/metrics), for the resources whose provider id starts with
   * one of `refs`, or all of them when unset.
   */
  charts?: {
    refs?: string[];
    /** Where the provider explains the plan its metrics need, when they need one. */
    plan?: string;
  };
  /**
   * Provider operations (by their names in its catalog) that give each request
   * it served, and daily requests by status, where it has them.
   */
  requests?: { logs: string; byStatus: string };
}

/** The domain changes a provider takes through Hoista. */
export interface DomainActions {
  add?: boolean;
  port?: boolean;
  rename?: boolean;
  certificate?: boolean;
  remove?: boolean;
}
