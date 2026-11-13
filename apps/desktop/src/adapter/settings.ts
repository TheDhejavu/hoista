/**
 * Settings on top of `hoistd`.
 *
 * Three owners, kept apart:
 * - the engine owns agent access, unused-agent expiry, build isolation,
 *   backups and update checks (`GET|PATCH /settings`, translated to its flat shape);
 * - the desktop owns appearance, startup and notification preferences, kept
 *   in a small file by the app;
 * - the remote comes from the engine's server status;
 * - everything else (the vault lock, approval timeout, what the leak check
 *   does) is not in the engine yet. Those show what actually happens today,
 *   and changing them is refused with a clear message instead of a save that
 *   does nothing.
 */
import type {
  DaemonStatus,
  Settings as DaemonSettings,
  SettingsPatch as DaemonPatch,
} from '@engine-api';
import type { RemoteStatus, Settings, SettingsPatch } from '@shared/api';
import { isServiceId } from '@shared/providers';
import { RouteError, type Router } from '@shared/router';
import type { Call } from './call';
import type { Desktop } from './desktop';
import { MINUTE_MS } from '@shared/time';

/** The app's own preferences, kept on this computer. */
export type DesktopPrefs = Settings['general'];

export interface PrefsStore {
  read(): DesktopPrefs;
  write(prefs: DesktopPrefs): void;
}

export const DEFAULT_PREFS: DesktopPrefs = {
  theme: 'system',
  open_at_login: 'menu_bar',
  notify: 'problems',
};

/** What the engine does today for settings it does not store. */
const FIXED = {
  approval_timeout_minutes: 15,
  on_safety_problem: 'stop',
  channel: 'stable',
} as const;

function unavailable(what: string): never {
  throw new RouteError('settings.unavailable', `${what} is not available in Hoista’s engine yet.`);
}

/** The engine's backup interval, as one of the choices the app offers. */
function backupHours(hours: number): Settings['backups']['every_hours'] {
  return ([0, 6, 24, 168] as const).find((choice) => choice === hours) ?? 24;
}

/** The engine's history length, as one of the choices the app offers. */
function historyDays(days: number): Settings['backups']['history_days'] {
  return ([30, 90, 365, 1095] as const).find((choice) => choice === days) ?? 365;
}

/** `GET /build-isolation`. */
interface BuildIsolation {
  isolated: boolean;
  how: string | null;
  why_not: string | null;
}

export function addSettingsRoutes(
  router: Router,
  call: Call,
  { prefs, findCli }: Pick<Desktop, 'prefs' | 'findCli'>,
  /** This computer's engine: the command line is this computer's, whichever Hoista is shown. */
  callLocal: Call = call,
): void {
  /**
   * The background engine on this computer, for the command line. Only a
   * local engine has one; while a remote is shown, the version is the
   * remote's, from its health check.
   */
  async function status(): Promise<DaemonStatus> {
    const here = await callLocal<DaemonStatus>('GET', '/daemon');
    if (callLocal === call) return here;
    const shown = await call<{ version: string }>('GET', '/health').catch(() => null);
    return { ...here, version: shown?.version ?? here.version };
  }

  async function read(): Promise<Settings> {
    const [engine, daemon, cli, isolation] = await Promise.all([
      call<DaemonSettings>('GET', '/settings'),
      status(),
      cliOnThisComputer(findCli),
      call<BuildIsolation>('GET', '/build-isolation').catch(() => null),
    ]);
    const settings = toSettings(engine, prefs.read(), daemon.version);
    settings.safety.builds_isolated_by = isolation?.how ?? null;
    settings.safety.builds_not_isolated_because = isolation?.why_not ?? null;
    settings.cli = withEngineCli(cli, daemon);
    return settings;
  }

  router.on('GET', '/settings', read);

  router.on('PATCH', '/settings', async ({ body }): Promise<Settings> => {
    const patch = (body ?? {}) as SettingsPatch;
    const enginePatch = toEnginePatch(patch);
    if (patch.general) prefs.write({ ...prefs.read(), ...patch.general });
    if (Object.keys(enginePatch).length > 0) await call('PATCH', '/settings', enginePatch);
    return read();
  });
}

/** Quotes one shell word when it needs it, so a pasted command runs as is. */
function shellWord(word: string): string {
  return /^[\w@%+=:,./-]+$/.test(word) ? word : `'${word.replace(/'/g, `'\\''`)}'`;
}

/**
 * The CLI as coding tools will use it. The engine names the `hoist` beside
 * it (with its state folder), which works even when `hoist` isn't on the
 * PATH; otherwise the PATH one is used.
 */
function withEngineCli(cli: Settings['cli'], daemon: DaemonStatus): Settings['cli'] {
  const command = daemon.mcp_command;
  if (command.length > 0) {
    return {
      ...cli,
      path: cli.installed ? cli.path : (command[0] ?? null),
      version: cli.version ?? daemon.version,
      mcp_command: command.map(shellWord).join(' '),
      mcp_args: command,
    };
  }
  return {
    ...cli,
    mcp_command: cli.installed ? 'hoist mcp' : null,
    mcp_args: cli.installed ? ['hoist', 'mcp'] : null,
  };
}

/** The last answer, kept a minute: settings are read on every engine event. */
let foundCli: { at: number; cli: Promise<Settings['cli']> } | null = null;

/** Whether `hoist` is installed on this computer, and its version. */
function cliOnThisComputer(findCli: Desktop['findCli']): Promise<Settings['cli']> {
  if (foundCli && Date.now() - foundCli.at < MINUTE_MS) return foundCli.cli;
  const cli = findCli()
    .catch(() => null)
    .then((found): Settings['cli'] => ({
      installed: found !== null,
      path: found?.path ?? null,
      version: found?.version ?? null,
      mcp_command: null,
      mcp_args: null,
    }));
  foundCli = { at: Date.now(), cli };
  return cli;
}

export function toSettings(engine: DaemonSettings, local: DesktopPrefs, version: string): Settings {
  return {
    general: local,
    agents: {
      connection_enabled: engine.agents_enabled,
      network_page: engine.network_page,
      mcp_http: engine.mcp_http,
      default_access: fromEngineAccess(engine.default_access),
      disconnect_unused_after_days: engine.agent_expiry_days,
      approval_timeout_minutes: FIXED.approval_timeout_minutes,
      remote: engine.server
        ? {
            target: remoteTarget(engine.server.provider ?? ''),
            status: 'running',
            last_sync: null,
            token_access: 'full',
            url: engine.server.url,
          }
        : {
            target: 'this_computer',
            status: 'off',
            last_sync: null,
            token_access: 'read_only',
            url: null,
          },
    },
    safety: {
      on_safety_problem: FIXED.on_safety_problem,
      builds_isolated_by: null,
      builds_not_isolated_because: null,
      build_without_isolation: engine.unisolated_builds,
    },
    backups: {
      every_hours: backupHours(engine.backup_every_hours),
      keep: engine.backup_keep,
      history_days: historyDays(engine.history_days),
    },
    updates: {
      channel: FIXED.channel,
      auto_check: engine.check_updates,
      current_version: version,
      latest_version: null,
      latest_summary: null,
      latest_notes: [],
      checked_at: null,
      can_update: false,
    },
    cli: { installed: false, path: null, version: null, mcp_command: null, mcp_args: null },
  };
}

/**
 * The engine half of a desktop patch. Throws for anything the engine cannot
 * store, before anything is written, so a mixed patch never half-applies.
 */
export function toEnginePatch(patch: SettingsPatch): Partial<DaemonPatch> {
  const out: Partial<DaemonPatch> = {};
  const agents = patch.agents ?? {};
  if (agents.connection_enabled !== undefined) out.agents_enabled = agents.connection_enabled;
  if (agents.network_page !== undefined) out.network_page = agents.network_page;
  if (agents.mcp_http !== undefined) out.mcp_http = agents.mcp_http;
  if (patch.backups?.every_hours !== undefined) out.backup_every_hours = patch.backups.every_hours;
  if (patch.backups?.keep !== undefined) out.backup_keep = patch.backups.keep;
  if (patch.backups?.history_days !== undefined) out.history_days = patch.backups.history_days;
  if (patch.safety?.build_without_isolation !== undefined) {
    out.unisolated_builds = patch.safety.build_without_isolation;
  }
  if (agents.disconnect_unused_after_days !== undefined) {
    if (agents.disconnect_unused_after_days === null) {
      throw new RouteError(
        'settings.unavailable',
        'Unused agents are disconnected after at most a year. Choose a number of days.',
      );
    }
    out.agent_expiry_days = agents.disconnect_unused_after_days;
  }
  if (agents.default_access !== undefined) {
    out.default_access = agents.default_access === 'look' ? 'view' : agents.default_access;
  }
  if (
    agents.approval_timeout_minutes !== undefined &&
    agents.approval_timeout_minutes !== FIXED.approval_timeout_minutes
  ) {
    unavailable('A longer approval timeout');
  }
  if (agents.remote !== undefined) unavailable('Changing the remote here');

  if (patch.safety?.on_safety_problem !== undefined && patch.safety.on_safety_problem !== 'stop') {
    unavailable('Continuing past a safety problem');
  }
  if (patch.updates?.auto_check !== undefined) out.check_updates = patch.updates.auto_check;
  if (patch.updates?.channel !== undefined && patch.updates.channel !== 'stable') {
    unavailable('The beta channel');
  }
  return out;
}

/** The engine's consent-page access (`view`, `ask`, `full`), as the app names it. */
function fromEngineAccess(access: string): Settings['agents']['default_access'] {
  if (access === 'view') return 'look';
  return access === 'full' ? 'full' : 'ask';
}

/** Where the server runs, as the app names it: the provider, or just "a server". */
function remoteTarget(provider: string): RemoteStatus['target'] {
  return isServiceId(provider) && provider !== 'github' ? provider : 'server';
}
