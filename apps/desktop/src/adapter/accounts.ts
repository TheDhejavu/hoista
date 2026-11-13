/**
 * Accounts on top of `hoistd`.
 *
 * Connecting is two engine calls: an unsaved lookup of which accounts the
 * token can reach, then a verified save of the chosen one. The desktop asks
 * for the lookup first (`POST /accounts/lookup`) and, when the token reaches
 * several accounts, lets the person choose; the engine never guesses. A token
 * travels only in these requests and is never read back.
 *
 * Replacing a token reconnects the same provider account, which the engine
 * verifies and saves under the same identity. Services without an engine
 * adapter (GitHub, Netlify) are reported as not available yet
 * instead of failing later.
 *
 * Connecting on the provider's own page (`POST /accounts/sign-in`) is offered only
 * for providers whose engine adapter has a Hoista OAuth app: the engine starts the
 * sign-in and returns the provider's page, this process opens it in the browser,
 * and the engine finishes once the person clicks Allow there. No token passes
 * through the desktop.
 */
import type {
  Account as DaemonAccount,
  AccountChoice,
  ProjectOverview,
  Provider,
} from '@engine-api';
import type {
  Account,
  CatalogueEntry,
  ConnectGuide,
  ConnectResult,
  Page,
  ServiceId,
  TokenCheck,
} from '@shared/api';
import { catalogue, connectGuides, type ConnectSteps } from '@shared/catalogue';
import { RouteError, type Router } from '@shared/router';
import { abilitiesOf, type Abilities, type ReadAbilities } from './abilities';
import { allPages, type Call } from './call';

/** Opens a provider's page in the person's browser. */
export type OpenPage = (url: string) => Promise<void>;

function engineProvider(service: unknown, abilities: Abilities): Provider {
  if (typeof service === 'string' && abilities.providers.has(service)) {
    return service as Provider;
  }
  throw new RouteError(
    'account.unsupported',
    'Hoista’s engine cannot connect this service yet. It is coming soon.',
  );
}

function field(body: unknown, name: string): string | undefined {
  if (typeof body !== 'object' || body === null || !(name in body)) return undefined;
  const value = (body as Record<string, unknown>)[name];
  return typeof value === 'string' ? value : undefined;
}

export function addAccountRoutes(
  router: Router,
  call: Call,
  openPage: OpenPage,
  abilities: ReadAbilities = abilitiesOf(call),
): void {
  async function allAccounts(): Promise<DaemonAccount[]> {
    return allPages<DaemonAccount>(call, '/accounts');
  }

  /**
   * Which projects each account's resources belong to, by account id; and
   * for GitHub, which repositories projects deploy from, by owner.
   */
  async function usedBy(): Promise<Map<string, string[]>> {
    const projects = await allPages<ProjectOverview>(call, '/projects/summaries');
    const names = new Map<string, Set<string>>();
    const add = (key: string, value: string) => {
      const set = names.get(key) ?? new Set<string>();
      set.add(value);
      names.set(key, set);
    };
    for (const overview of projects) {
      for (const resource of overview.resources) add(resource.account_id, overview.project.name);
      const source = overview.project.source;
      const repo =
        source.kind === 'github' ? source.repository : (overview.source_git?.repository ?? null);
      const owner = repo?.split('/')[0];
      if (repo && owner) add(`github:${owner.toLowerCase()}`, repo);
    }
    return new Map([...names].map(([id, set]) => [id, [...set].sort()]));
  }

  /** What an account is used by: projects, or for GitHub its repositories. */
  function usedFor(account: DaemonAccount, used: Map<string, string[]>): string[] {
    if (account.provider === 'github') {
      return (
        used.get(`github:${account.label.toLowerCase()}`) ??
        used.get(`github:${account.external_id.toLowerCase()}`) ??
        []
      );
    }
    return used.get(account.id) ?? [];
  }

  async function connect(
    provider: Provider,
    token: string,
    choice: { external_id: string; label: string },
  ): Promise<ConnectResult> {
    const saved = await call<DaemonAccount>('POST', '/accounts', {
      provider,
      label: choice.label,
      external_id: choice.external_id,
      token,
    });
    return { account: await shown(saved, []), checks: checksFor(saved) };
  }

  /** An account as the app shows it: watched where Hoista can't deploy to its provider. */
  async function shown(account: DaemonAccount, projects: string[]): Promise<Account> {
    return toAccount(account, projects, (await abilities()).can(account.provider, 'deploy'));
  }

  router.on('GET', '/accounts', async (): Promise<Page<Account>> => {
    const [accounts, projects, known] = await Promise.all([allAccounts(), usedBy(), abilities()]);
    return {
      items: accounts.map((account) =>
        toAccount(account, usedFor(account, projects), known.can(account.provider, 'deploy')),
      ),
      cursor: null,
    };
  });

  router.on('GET', '/accounts/:id', async ({ params }): Promise<Account> => {
    const [accounts, projects] = await Promise.all([allAccounts(), usedBy()]);
    const account = accounts.find((candidate) => candidate.id === params.id);
    if (!account) throw new RouteError('account.not_found', 'This account is no longer connected.');
    return shown(account, usedFor(account, projects));
  });

  router.on('POST', '/accounts/lookup', async ({ body }): Promise<AccountChoice[]> => {
    const provider = engineProvider(field(body, 'service'), await abilities());
    return call<AccountChoice[]>('POST', '/accounts/lookup', {
      provider,
      token: field(body, 'token') ?? '',
    });
  });

  router.on('POST', '/accounts', async ({ body }): Promise<ConnectResult> => {
    const provider = engineProvider(field(body, 'service'), await abilities());
    const token = field(body, 'token') ?? '';
    const externalId = field(body, 'external_id');
    if (externalId) {
      return connect(provider, token, {
        external_id: externalId,
        label: field(body, 'label') ?? externalId,
      });
    }
    const choices = await call<AccountChoice[]>('POST', '/accounts/lookup', { provider, token });
    const [only] = choices;
    if (choices.length !== 1 || !only) {
      throw new RouteError(
        'account.choice_required',
        choices.length === 0
          ? 'This token cannot list its accounts. Enter the account ID.'
          : 'This token reaches more than one account. Choose one.',
      );
    }
    return connect(provider, token, { external_id: only.external_id, label: only.name });
  });

  router.on('PUT', '/accounts/:id/token', async ({ params, body }): Promise<ConnectResult> => {
    const account = (await allAccounts()).find((candidate) => candidate.id === params.id);
    if (!account) throw new RouteError('account.not_found', 'This account is no longer connected.');
    return connect(account.provider, field(body, 'token') ?? '', {
      external_id: account.external_id,
      label: account.label,
    });
  });

  router.on('POST', '/accounts/sign-in', async ({ body }): Promise<ConnectResult> => {
    const provider = engineProvider(field(body, 'service'), await abilities());
    if (!(await abilities()).can(provider, 'browser_sign_in')) {
      throw new RouteError(
        'account.sign_in_unavailable',
        'This provider connects with a token for now. Use a token instead.',
      );
    }
    const started = await call<{ id: string; url: string }>('POST', '/accounts/sign-in', {
      provider,
    });
    await openPage(started.url);
    // Waits until the person clicks Allow (or declines) on the provider's page.
    const saved = await call<DaemonAccount>(
      'POST',
      `/accounts/sign-in/${encodeURIComponent(started.id)}/finish`,
    );
    const checks = checksFor(saved);
    checks[0] = { label: `Signed in on ${connectGuides[provider].name}`, ok: true, note: null };
    return { account: await shown(saved, []), checks };
  });

  // Only services with an engine adapter can be connected; the rest are coming soon.
  router.on('GET', '/catalogue', async (): Promise<Page<CatalogueEntry>> => {
    const known = await abilities({ strict: true });
    return {
      items: catalogue.map((entry) =>
        entry.service && known.providers.has(entry.service)
          ? entry
          : { ...entry, availability: 'coming_soon' },
      ),
      cursor: null,
    };
  });

  router.on('GET', '/catalogue/:id/connect', async ({ params }): Promise<ConnectGuide> => {
    const guide = connectGuides[params.id as ServiceId] as ConnectSteps | undefined;
    // An engine that didn't answer says so, not that the service has no guide.
    const known = await abilities({ strict: true });
    if (!guide) {
      throw new RouteError('catalogue.not_found', 'There is no guide for this service.');
    }
    const links = known.links(guide.service);
    if (!links) {
      // The app knows the provider but the engine running didn't list it:
      // usually an older hoistd still running after an update.
      throw new RouteError(
        'catalogue.not_found',
        `Hoista’s engine didn’t list ${guide.name}. If you updated Hoista, restart hoistd and try again.`,
      );
    }
    return {
      ...guide,
      signup_url: links.signup,
      token_url: links.tokens,
      // Only providers the engine can sign in to offer connecting on their page.
      sign_in: known.can(guide.service, 'browser_sign_in') ? guide.sign_in : null,
    };
  });
}

const STATUS: Record<DaemonAccount['status'], Account['status']> = {
  connected: 'ok',
  expired: 'expired',
  missing_scopes: 'missing_scope',
  disconnected: 'revoked',
};

const PROBLEM: Record<Exclude<DaemonAccount['status'], 'connected'>, string> = {
  expired:
    'The token expired or was revoked at the provider. Replace it to keep managing projects.',
  missing_scopes: 'The token is missing permissions Hoista needs. Make a new one with them.',
  disconnected: 'The token was removed from this computer. Projects keep running at the provider.',
};

/** `deploys`: Hoista deploys to this provider; otherwise it only watches what's there. */
export function toAccount(account: DaemonAccount, usedBy: string[], deploys = true): Account {
  const scan = account.last_discovery;
  return {
    id: account.id,
    service: account.provider,
    label: account.label,
    identity: account.external_id,
    // The engine never exposes any part of a saved token.
    token_hint: '',
    kept_in: account.in_1password ? '1password' : 'keychain',
    scopes: account.scopes,
    missing_scopes: [],
    status: STATUS[account.status],
    problem:
      account.status === 'connected' ? null : { since: null, message: PROBLEM[account.status] },
    // Providers Hoista can only discover on are watched; nothing is managed there.
    mode: deploys ? 'managing' : 'watching',
    connected_at: account.connected_at,
    last_worked_at: scan && scan.observed_resources !== null ? scan.observed_at : null,
    used_by: usedBy,
    scan_note: scanNote(scan),
  };
}

/** Providers' names for the kinds of things a scan reads. */
const KINDS: Record<string, string> = {
  workers: 'Workers',
  pages: 'Pages projects',
  zones: 'domains',
};

function kindName(kind: string): string {
  if (kind.startsWith('r2')) return 'R2 buckets';
  return KINDS[kind] ?? kind;
}

/** What the last scan missed, or null when it read everything. */
export function scanNote(scan: DaemonAccount['last_discovery']): string | null {
  if (!scan) return null;
  if (scan.observed_resources === null) {
    return 'Hoista couldn’t finish looking through this account, so some projects may be missing. It tries again on its own.';
  }
  if (scan.complete) return null;
  const missed = [...new Set(scan.warnings.map((warning) => kindName(warning.resource_type)))];
  return missed.length > 0
    ? `Hoista couldn’t read ${missed.join(', ')} on this account, so those aren’t listed. The sign-in or token may not include them.`
    : 'Some things on this account couldn’t be read, so they aren’t listed.';
}

/** What the engine established when it saved the token. */
export function checksFor(account: DaemonAccount): TokenCheck[] {
  const checks: TokenCheck[] = [{ label: 'Token works', ok: true, note: null }];
  const scan = account.last_discovery;
  if (scan) {
    checks.push(
      scan.observed_resources === null
        ? {
            label: 'Read your existing projects',
            ok: false,
            note: 'The scan did not finish. Hoista retries; your projects are not affected.',
          }
        : {
            label: `Found ${String(scan.observed_resources)} existing resources`,
            ok: scan.complete,
            note: scan.complete ? null : 'Some resource types could not be read with this token.',
          },
    );
  }
  return checks;
}
