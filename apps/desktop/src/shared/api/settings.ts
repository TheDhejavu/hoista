/**
 * Project-wide settings, the Needs attention panel and live engine events.
 *
 * Endpoints: `GET|PATCH /settings`, `POST /diagnostics`, `GET /attention`,
 * `GET /events` (SSE).
 */
import type { EventDto } from '@engine-api';
import type { ProviderId, Timestamp } from './core';
import type { ServiceId } from './accounts';
import type { AccessLevel, WaitingOnAccount } from './agents';

/** `GET /settings`. `PATCH /settings` takes any subset, section by section. */
export interface Settings {
  general: {
    theme: 'dark' | 'light' | 'system';
    /** `menu_bar` keeps Hoista running in the menu bar so agents can reach it. */
    open_at_login: 'menu_bar' | 'window' | 'off';
    notify: 'deploys' | 'problems' | 'never';
  };
  agents: {
    /** Whether agents can reach Hoista at all. */
    connection_enabled: boolean;
    /** Also open Hoista's page on this computer's Wi-Fi address. Off by default; applies when Hoista starts. */
    network_page: boolean;
    /**
     * Serve MCP over HTTP on `127.0.0.1:8788`, for web agents such as ChatGPT
     * through a tunnel or proxy the person runs. On by default.
     */
    mcp_http: boolean;
    default_access: AccessLevel;
    /** Null means never. */
    disconnect_unused_after_days: number | null;
    /** An unanswered approval counts as no after this long. */
    approval_timeout_minutes: 15 | 60;
    /** Where Hoista keeps running when this computer is off. */
    remote: RemoteStatus;
  };
  safety: {
    /** What the leak check does when it finds a problem. `ask` waits for your OK. */
    on_safety_problem: 'stop' | 'ask' | 'warn';
    /**
     * What keeps a project's build code away from the rest of this computer:
     * its name (`bubblewrap`, `the macOS sandbox`), or null when there's none.
     */
    builds_isolated_by: string | null;
    /** Why builds can't be isolated here, when they can't. */
    builds_not_isolated_because: string | null;
    /** Build anyway where they can't be isolated. Off unless the person turns it on. */
    build_without_isolation: boolean;
  };
  backups: {
    /** Hours between automatic backups; 0 means none. */
    every_hours: 0 | 6 | 24 | 168;
    /** How many are kept. */
    keep: number;
    /** Days of activity and finished history kept; at least 30. */
    history_days: 30 | 90 | 365 | 1095;
  };
  updates: {
    channel: 'stable' | 'beta';
    auto_check: boolean;
    current_version: string;
    latest_version: string | null;
    /** One line about what the latest version adds. */
    latest_summary: string | null;
    /** A few short lines on what changed, for the update dialog. */
    latest_notes: string[];
    checked_at: Timestamp | null;
    /** Whether this copy can update itself: a signed release with an update feed. */
    can_update: boolean;
  };
  cli: {
    /** `hoist` runs from this computer's PATH. */
    installed: boolean;
    /** Where the `hoist` Hoista uses lives, on the PATH or beside the engine. */
    path: string | null;
    version: string | null;
    /**
     * The command a coding tool runs to reach this Hoista over MCP, ready to
     * paste (paths quoted), or null when Hoista can't find its `hoist` binary.
     */
    mcp_command: string | null;
    /** The same command as its words, for tools set up with a JSON file. */
    mcp_args: string[] | null;
  };
}

/** A partial update for `PATCH /settings`. */
export type SettingsPatch = { [Section in keyof Settings]?: Partial<Settings[Section]> };

/**
 * Where the remote runs: a provider account, a server of the person's
 * own, or `this_computer` when there is none.
 */
export type RemoteTarget = 'this_computer' | ProviderId | 'server';

/** Where agents reach Hoista: this computer or a remote, carried in settings. */
export interface RemoteStatus {
  target: RemoteTarget;
  status: 'off' | 'setting_up' | 'running' | 'unreachable';
  /** When the remote last synced with this computer. */
  last_sync: Timestamp | null;
  /** What the remote may do with account tokens. Read-only unless the user widens it. */
  token_access: 'read_only' | 'full';
  /** The server's address (`https://host`) when Hoista runs on one. */
  url?: string | null;
}

/**
 * `POST /server`: put Hoista on one of the person's provider accounts. It runs
 * the published image this release names, pinned by its digest.
 */
export interface ServerRequest {
  account_id: string;
}

/** `GET /server/providers`: the providers Hoista can run on, by what their adapters support. */
export interface ServerProviders {
  providers: Exclude<RemoteTarget, 'this_computer' | 'server'>[];
}

/** What `POST /server` hands back. */
export interface ServerSetup {
  url: string;
  /** Opens the claim page with the one-time code after `#`. Open it once. */
  claim_link: string;
  /** Whether the provider now hides the server's sealing key from its dashboard, API and CLI. */
  key_hidden: boolean;
  /** What to do at the provider when it doesn't. */
  seal_help?: string;
  /** The provider's page for that. */
  seal_page?: string;
}

/** One of your remotes: another Hoista the app can show, on a server of yours. */
export interface RemoteItem {
  id: string;
  /** What you call it, such as `home-server`. */
  name: string;
  /** Its address: `https://host`, or `http://localhost:PORT` for one in Docker on this Mac. */
  url: string;
  added_at: Timestamp;
  /** This computer has a key for it. */
  signed_in: boolean;
  /** It answered its health check just now; null when not checked. */
  reachable: boolean | null;
}

/**
 * `GET /remotes`: your remotes and which Hoista the app shows. `POST /remotes`
 * adds one by its address, `PATCH`/`DELETE /remotes/{id}` renames or forgets
 * one here, `POST`/`DELETE /remotes/{id}/sign-in` signs this computer in or
 * out, `PUT /remotes/showing` switches, `POST /remotes/{id}/push` moves this
 * computer's projects there. Desktop only.
 */
export interface RemotesView {
  /** `this_computer`, or the id of the remote every screen shows. */
  showing: string;
  items: RemoteItem[];
  /** A sign-in waiting for the person in the browser: Allow on that remote's page. */
  waiting: { remote_id: string; link: string } | null;
  /** Why the last sign-in to that remote didn't finish. */
  problem: { remote_id: string; message: string } | null;
}

/** `GET /backups`, newest first, and `POST /backups` (back up now). */
export interface BackupFile {
  name: string;
  at: Timestamp;
  bytes: number;
}

/**
 * `GET /remotes/{id}/contents`: what a signed-in remote and this computer
 * hold. A push goes only into a remote with no projects or accounts yet.
 */
export interface RemoteContents {
  remote: { projects: number; accounts: number };
  here: { projects: number; accounts: number };
}

/** `POST /remotes/{id}/push`: what moved to the remote. */
export interface ServerMove {
  projects: number;
  /** Provider sign-ins restored on the server. */
  sign_ins: number;
  /** Projects that stayed watching on the server: switch them to managed there. */
  left_watching: string[];
}

/** `POST /diagnostics`: a report with no tokens or secret values, saved on this computer. */
export interface DiagnosticReport {
  path: string;
}

/**
 * `POST /transfer/export`: packs everything Hoista knows into one file,
 * encrypted with a passphrase the user picks, for moving to a new computer.
 */
export interface ExportTransfer {
  /** Where to save it: an absolute path the person chose in the save dialog. */
  path: string;
  passphrase: string;
}

export interface TransferFile {
  /** Where the file was saved, for example `~/Downloads/sam.hoista`. */
  path: string;
  created_at: Timestamp;
  /** What's inside, in plain words, for example `13 projects and their maps`. */
  contents: string[];
}

/** `POST /transfer/import`: restores a transfer file on this computer. */
export interface ImportTransfer {
  path: string;
  passphrase: string;
}

export interface TransferImported {
  profile_name: string;
  projects: number;
  accounts: number;
}

/** One item in the Needs attention panel. */
export interface AttentionItem {
  id: string;
  kind: 'saving' | 'failure' | 'approval' | 'account' | 'unclear';
  /** Drives the dot colour: good = green, bad = red, warn = amber. */
  tone: 'good' | 'bad' | 'warn';
  project_id: string | null;
  title: string;
  body: string;
  approval_id: string | null;
  /** The agent asking, for approvals, so its logo can be shown. */
  agent: string | null;
  /** For account items: the agent request waiting for you to reconnect, if any. */
  waiting_on: WaitingOnAccount | null;
  /**
   * For unclear items: the change Hoista didn't hear back about, which the
   * person checks at the provider and answers with `POST /operations/{id}/check`.
   */
  operation?: { id: string; service: ServiceId; target: string; started_at: Timestamp };
}

/**
 * One entry from the engine's activity stream (`GET /events`), exactly as the
 * daemon sends it. `kind` says what happened (`deploy.step`, `approval.created`…),
 * `subject_id` names the record it is about, and the UI refetches that record
 * rather than reading details from the event. The UI subscribes once and
 * never polls providers.
 */
export type EngineEvent = EventDto;
