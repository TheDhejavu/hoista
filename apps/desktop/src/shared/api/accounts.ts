/**
 * The user's profile, connected accounts and the catalogue of services.
 *
 * Endpoints: `GET|PUT /profile`, `GET|POST /accounts`, `DELETE /accounts/{id}`.
 * Proposed, not yet in the backend spec: `GET /accounts/{id}`,
 * `PUT /accounts/{id}/token` (replace a token that stopped working),
 * `GET /catalogue` and `GET /catalogue/{id}/connect` (how to make a token).
 *
 * Tokens go from the connect form straight to the engine, which stores them in
 * the system keychain. No endpoint ever returns a token.
 */
import type { Capabilities } from '@engine-api';
import type { ResourceKind } from './projects';
import type { Money, ProviderId, Timestamp } from './core';

/** The one local profile. There is no sign-in and no Hoista account. */
export interface Profile {
  name: string;
  initials: string;
}

/** `PUT /profile` body. The engine works out the initials. */
export interface ProfileUpdate {
  name: string;
}

/** Services an account can be connected for: the hosting providers plus GitHub. */
export type ServiceId = ProviderId | 'github';

/** One connected account. */
export interface Account {
  id: string;
  service: ServiceId;
  /** What the user calls it, defaulting to the account name at the service. */
  label: string;
  /** Account name or email at the service. */
  identity: string;
  /** The last four characters of the token, so the user can tell tokens apart. Empty when not kept. */
  token_hint: string;
  /** Where the token is: this computer's keychain, or 1Password (Hoista keeps only its reference). */
  kept_in: 'keychain' | '1password';
  scopes: string[];
  /** Scopes an adapter needs that this token lacks, by name. */
  missing_scopes: string[];
  status: 'ok' | 'expired' | 'missing_scope' | 'revoked';
  /** Set when the token stopped working: since when (null if unknown), and what it means. */
  problem: { since: Timestamp | null; message: string } | null;
  /** `managing` for normal accounts; `watching` when everything on it was imported read-only. */
  mode: 'managing' | 'watching';
  connected_at: Timestamp;
  last_worked_at: Timestamp | null;
  /** Names of the projects (or repos, for GitHub) that use this account. */
  used_by: string[];
  /** Set when Hoista's last look at this account didn't read everything: what was missed, in plain words. */
  scan_note?: string | null;
}

/**
 * `POST /accounts` body. The token is sent once and never read back. Name
 * the account (from `POST /accounts/lookup`) when the token reaches several.
 */
export interface ConnectAccount {
  service: ServiceId;
  token: string;
  external_id?: string;
  label?: string;
}

/** `POST /accounts/lookup` body: which accounts can this token reach? Nothing is saved. */
export interface LookupAccounts {
  service: ServiceId;
  token: string;
}

/** One account a token can reach, from `POST /accounts/lookup`. */
export interface AccountChoice {
  external_id: string;
  name: string;
}

/**
 * `POST /accounts/sign-in` body: connect by approving Hoista on the
 * service's own page instead of pasting a token. The engine opens the
 * browser, waits for the approval and keeps the token it gets back.
 */
export interface SignInAccount {
  service: ServiceId;
}

/** `PUT /accounts/{id}/token` body. */
export interface ReplaceToken {
  token: string;
}

/** One check the engine ran on a new token. */
export interface TokenCheck {
  label: string;
  ok: boolean;
  /** For a failed check: what it means and whether it matters now. */
  note: string | null;
}

/** `POST /accounts` and `PUT /accounts/{id}/token` result. */
export interface ConnectResult {
  account: Account;
  checks: TokenCheck[];
}

/** Groups in the catalogue. */
export type Category =
  'hosting' | 'gpu' | 'databases' | 'storage' | 'email' | 'sign_in' | 'domains' | 'code';

/** One service in the catalogue, connectable or not yet. */
export interface CatalogueEntry {
  /** For example `cloudflare`, `resend`, `planetscale`. */
  id: string;
  name: string;
  /** What Hoista uses it for, in a few words. */
  description: string;
  category: Category;
  /** Set for services that have an adapter today. */
  service: ServiceId | null;
  availability: 'available' | 'coming_soon';
  /** Why it is recommended for the user's projects, if it is. */
  recommended_because: string | null;
}

/** `GET /catalogue/{id}/connect`: how to make a token for one service. */
export interface ConnectGuide {
  service: ServiceId;
  name: string;
  /** One or two sentences on what connecting does. */
  intro: string;
  /** The provider's pages for making an account and a token, from the engine. */
  signup_url: string;
  token_url: string;
  /** Where to click at the service, for example `Account → Access Tokens → Generate new token`. */
  token_steps: string;
  token_label: string;
  /** What a token looks like, for example `sbp_…`. */
  token_placeholder: string;
  /** Permissions to add, for services with fine-grained tokens. */
  permissions: TokenPermission[];
  /** Set for services that let you approve Hoista on their own page. */
  sign_in: SignInOption | null;
}

/** Connecting by approving Hoista on the service's own page. */
export interface SignInOption {
  /** What happens on their page, for example `You pick which projects Hoista can see.` */
  on_their_page: string;
}

/** One row of a token permission table, as the service's own form lays it out. */
export interface TokenPermission {
  scope: string;
  resource: string;
  access: string;
  /** Only needed for some features, for example custom domains. */
  optional_for: string | null;
}

/**
 * What a provider has actually billed one account so far this billing period
 * (`GET /bills`). Hoista never estimates cost: providers that give no bill, and
 * per-project amounts, are simply absent.
 */
export interface AccountBill {
  account_id: string;
  provider: ProviderId;
  label: string;
  bill: {
    billed: Money;
    period_start: Timestamp | null;
    period_end: Timestamp | null;
    /** The provider's own usage limits, as set at the provider. */
    soft_limit: Money | null;
    hard_limit: Money | null;
    over_limit: boolean;
    /** The provider's customer id, needed to change the limits. Not a secret. */
    customer_id?: string | null;
  };
}

/** One thing a provider counts against the plan, such as emails sent this month. */
export interface ProviderPlanUsage {
  /** In the provider's words, such as `Emails this month`. */
  label: string;
  used: number;
  /** The plan's limit, when it has one. */
  limit: number | null;
  /** What is counted, such as `emails` or `GB`. */
  unit: string;
}

/**
 * Billing for one connected account (`GET /billing`): the provider's bill when it
 * gives one, its plan usage when it only shares usage, and a note otherwise.
 */
export interface AccountBilling {
  account_id: string;
  provider: ProviderId;
  label: string;
  bill: AccountBill['bill'] | null;
  plan_usage: ProviderPlanUsage[];
  /** Why there is no bill, in a sentence, when there isn't one. */
  note: string | null;
}

/** How much a provider operation can change. `destroy` asks first. */
export type ProviderRisk = 'read' | 'change' | 'destroy';

/** One argument of a provider operation. */
export interface ProviderOperationArg {
  name: string;
  /** The caller must give it; Hoista fills in `filled_in` ones itself. */
  required: boolean;
  /** `project`, `environment`, `service` or `workspace`, filled from the resource. */
  filled_in: string | null;
  /** JSON schema of the value. */
  schema: unknown;
}

/**
 * Something a provider can do (`GET /resources/{id}/provider/operations` or
 * `/accounts/{id}/provider/operations`): the provider's whole API, grouped by area.
 * Run with `POST …/provider/operations/{name}` and `{ args }`.
 */
export interface ProviderOperation {
  name: string;
  area: string;
  risk: ProviderRisk;
  /**
   * What it is in the provider's own terms: the HTTP method (`GET`, `DELETE`) of
   * a REST call, `Query` or `Mutation` for GraphQL, `Command` for its CLI.
   */
  request: string;
  description: string;
  args: ProviderOperationArg[];
}

/**
 * A button for a provider's resource, from the provider's manifest
 * (`GET /resources/{id}/provider/quick-actions`): one operation with its arguments
 * set. `try_first` are the arguments of a run that changes nothing (a dry
 * run), offered before the real one.
 */
export interface ProviderQuickAction {
  label: string;
  /** The kind of resource it's for; null for every kind. */
  on: ResourceKind | null;
  operation: string;
  description: string;
  args: Record<string, unknown>;
  try_first: Record<string, unknown> | null;
}

/** What a provider operation runs on. */
export type ProviderTarget = { kind: 'resource' | 'account'; id: string };

/** What one provider's engine adapter can do, from `GET /providers`. */
export interface ProviderAbility {
  provider: ProviderId;
  name: string;
  /** What the provider calls the account a token belongs to: `workspace ID`. */
  account_label: string;
  links: ProviderLinks;
  capabilities: Capabilities;
}

/** The provider's pages for making an account and a token, as the engine declares them. */
export interface ProviderLinks {
  signup: string;
  tokens: string;
}

/** `GET /providers`: every provider the engine has an adapter for. */
export interface ProviderAbilities {
  items: ProviderAbility[];
}

/** One thing a provider's adapter can or can't do. */
export type Ability = keyof Capabilities;
