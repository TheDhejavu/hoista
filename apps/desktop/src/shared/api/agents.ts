/**
 * AI agents connected over MCP and the approvals they ask for.
 *
 * Endpoints: `GET /agents`, `PATCH /agents/{id}` (proposed: change access),
 * `DELETE /agents/{id}`, `GET /agents/activity` (proposed),
 * `GET /agents/requests` and `POST /agents/requests/{id}` (proposed: new chat
 * projects asking to connect), `GET /approvals`, `GET /approvals/{id}` (proposed),
 * `POST /approvals/{id}`.
 */
import type { ServiceId } from './accounts';
import type { ProviderId, Timestamp } from './core';

/**
 * What an agent may do. Hoista enforces it on every call.
 */
export type AccessLevel =
  /** See projects, logs, deploys, metrics and bills. Can't change anything. */
  | 'look'
  /** Change anything; asks in the agent before deleting, removing or connecting an account. */
  | 'ask'
  /** Change anything, including deleting, without asking. */
  | 'full';

/** Stricter than an agent's access, in one environment: ask first, or only look. */
export type StricterAccess = 'ask' | 'look';

/** Events Hoista can send an agent, by MCP notification or signed webhook. */
type AgentEventType =
  'deploy.finished' | 'approval.created' | 'approval.resolved' | 'resource.status';

/** Where a background agent hears about events while nobody is chatting to it. */
export interface AgentWebhook {
  url: string;
  events: AgentEventType[];
  /** The last delivery, so a broken webhook is visible. */
  last_delivery: { at: Timestamp; ok: boolean } | null;
}

/** One connected agent. */
export interface Agent {
  id: string;
  name: string;
  /**
   * Chat apps and cloud agents reach Hoista through the Hoista URL (a remote);
   * coding tools run `hoist mcp` on this computer. Cloud agents run unattended,
   * so they get stricter defaults and can have a webhook.
   */
  kind: 'chat_app' | 'coding_tool' | 'cloud_agent';
  transport: 'stdio' | 'link';
  access: AccessLevel;
  /** The projects it may touch, by id. `null` means every project. */
  projects: string[] | null;
  webhook: AgentWebhook | null;
  last_seen: Timestamp | null;
  /** Hoista environments (ids) whose projects it may touch; null means every one. */
  environment_ids?: string[] | null;
  /** Stricter access by Hoista environment id; environments left out follow `access`. */
  environment_rules?: Record<string, StricterAccess>;
  /** Narrower access for some resources; the strictest that applies wins. */
  exceptions?: AgentException[];
  /** Tools it may never call, whatever its access. */
  never?: string[];
  /** When its access ends; null means no end date. */
  access_ends_at?: Timestamp | null;
  /** `waiting` means it has an approval waiting on the user. */
  status: 'active' | 'waiting' | 'idle';
}

/** The kinds of resource an exception can name together. */
export type ResourceGroup = 'services' | 'databases' | 'storage' | 'domains';

/**
 * One exception to an agent's access: a group of resources (in every project
 * or one) or a single resource, and how far it narrows access there. It
 * never gives more than the agent's access.
 */
export interface AgentException {
  /** A group of resources, or null for one resource. */
  group: ResourceGroup | null;
  /** With a group, the project it applies in (null for every project); with a resource, its project. */
  project_id: string | null;
  /** One resource, or null for a group. */
  resource_id: string | null;
  /** Ask first, View only, or hidden from it. */
  access: 'ask' | 'look' | 'hidden';
}

/** `GET /agents/activity/{id}`: what one call sent and how it ended. */
export interface AgentCall {
  /** The call as the agent sent it, pretty-printed. Secret values are never kept. */
  sent: string | null;
  /** How it ended in a word or two, such as `Completed` or `Refused`. */
  status: string | null;
  /** What the provider or Hoista said about it, one line each; empty when nothing more is known. */
  happened: string[];
  /** The approval it asked for, when it needed one. */
  approval_id: string | null;
}

/** `PATCH /agents/{id}` body. Only the fields sent change. */
export interface AgentUpdate {
  access?: AccessLevel;
  projects?: string[] | null;
  environment_ids?: string[] | null;
  environment_rules?: Record<string, StricterAccess>;
  exceptions?: AgentException[];
  never?: string[];
  access_ends_at?: Timestamp | null;
  webhook?: Omit<AgentWebhook, 'last_delivery'> | null;
}

/** One thing an agent did, for the Recent activity list. */
export interface AgentActivity {
  id: string;
  at: Timestamp;
  agent_id: string;
  agent_name: string;
  /** Plain words after the agent's name, for example `deployed moodboard`. */
  summary: string;
  /** The project it was about, if any. */
  project_id: string | null;
  /** The MCP tool it called, for example `deploy`. */
  tool: string;
  /** A short second line: version, providers, how long it took. */
  detail: string | null;
  tone: 'good' | 'bad' | 'warn' | 'neutral';
  /** Where to go to deal with it, if anything is needed from the user. */
  action: { label: string; project_id: string; tab: 'env' | 'deploys' | 'logs' } | null;
  /** How it ended, in a few words, for example `Live · 41s` or `Waiting for you`. */
  result: string;
  /** True for calls that only looked at something; the Agents page groups these. */
  read_only: boolean;
  /** The agent's own words for why, when it gave any. */
  reason: string | null;
  /** The tool call as the agent sent it, shown in code. Secrets are never included. */
  sent: string | null;
  /** What Hoista did next, one line each, oldest first. */
  happened: string[];
  /** The approval behind it, when it needed one. */
  approval_id: string | null;
  /** Set while the request waits for you to reconnect a provider Hoista can no longer reach. */
  waiting_on: WaitingOnAccount | null;
}

/**
 * An agent's request paused because Hoista's access to a provider stopped
 * working. It carries on by itself once you reconnect.
 */
export interface WaitingOnAccount {
  service: ServiceId;
  agent: string;
  project_id: string | null;
  /** The tool it waits to run, for example `deploy`. */
  action: string;
}

/** A chat app that came in through the private link and wants to connect. */
export interface ConnectionRequest {
  id: string;
  /** The name the app gave. Hoista cannot verify it. */
  claimed_name: string;
  /** A chat app through the Hoista URL, or a coding tool on this computer. */
  via: 'link' | 'this_computer';
  access: AccessLevel;
  created_at: Timestamp;
  /** The code the tool shows, to match the request with it. */
  code?: string | null;
  /** After this it can't be allowed. */
  expires_at?: Timestamp | null;
  /**
   * Where it's answered, the one place: a sign-in through the browser (OAuth)
   * on Hoista's page there, since that page sends the browser back to the
   * agent; a tool on this computer here, in the app.
   */
  answer_in: 'browser' | 'app';
  /** Hoista's page for it in the browser. */
  page?: string | null;
  /** How many earlier connections from the same tool on this computer allowing it replaces. */
  replaces?: number;
}

/** `POST /agents/requests/{id}` and `POST /approvals/{id}` body. */
export interface Decision {
  decision: 'allow' | 'decline';
  /** For a connection request: what it may do once allowed. */
  access?: AccessLevel;
  /** For a connection request: the projects it may reach; every project when left out. */
  projects?: string[] | null;
}

/** Something an approval would delete or change. */
export interface ApprovalTarget {
  /** What it is, for example `Project`, `Database`, `Address`. */
  label: string;
  name: string;
  provider: ProviderId;
}

/** An action an agent wants to take that needs the user's OK. */
export interface Approval {
  id: string;
  agent_id: string;
  agent_name: string;
  /** How the agent is connected and what it may do, for example `Connected by private link`. */
  agent_detail: string;
  /** For example `delete_app`, `delete_resource`, `delete_domain`, `connect_account`. */
  action: string;
  /** Plain-English summary, for example `ChatGPT wants to delete recipe-swap`. */
  title: string;
  /** The agent's own words for why it is asking. */
  reason: string;
  targets: ApprovalTarget[];
  /** Reviewed before/after settings from the daemon. */
  details?: { label: string; value: string }[];
  if_allowed: string;
  if_declined: string;
  /** When set, the user must type this to allow, for example the project's name. */
  confirm_text: string | null;
  created_at: Timestamp;
  expires_at: Timestamp;
  decision: 'allow' | 'decline' | 'expired' | 'consumed' | null;
  /** Server-projected reason this request cannot safely be approved in Desktop. */
  allow_blocked?: string | null;
  /** Exact reviewed impact sent only after the user types the confirmation name. */
  confirmation?: {
    backup_databases: boolean;
    confirmation_name: string;
    fingerprint: string;
  } | null;
}

/**
 * `GET /agents/link`: MCP over HTTP, for web agents such as ChatGPT through a
 * tunnel or proxy the person runs. `DELETE` forgets its public address.
 */
export interface AgentLink {
  /** Whether Hoista is listening on the port now. */
  listening: boolean;
  /** Where a tunnel or proxy points: `http://127.0.0.1:8788`. */
  local: string;
  /** Its public address: set, or learned from the first request from outside. */
  url: string | null;
  /** The address to give a web agent: the public one, with `/mcp`. */
  mcp: string | null;
  /** When a request last came in through the tunnel or proxy, since Hoista started. */
  seen_at?: Timestamp | null;
}

/** `GET /agents/address`: where agents connect to Hoista; null before the engine has started its page. */
export interface AgentAddress {
  /** `http://127.0.0.1:PORT`: this computer only. Null on a server. */
  local: string | null;
  /** `http://192.168.x.y:PORT` on the current network, when there is one. */
  network: string | null;
  /** Hoista on a server: its own address (`https://host`); agents add `/mcp`. */
  public?: string | null;
}
