import { describe, expect, it } from 'vitest';
import type { Account as DaemonAccount } from '@engine-api';
import type { CatalogueEntry, ConnectResult, EngineRequest, Page } from '@shared/api';
import { unwrap, type Engine } from '@shared/engine';
import { checksFor, scanNote, toAccount } from './accounts';
import type { Capabilities } from '@engine-api';
import type { ProviderAbilities } from '@shared/api';
import { createAdapter } from '.';

const none: Capabilities = {
  deploy: false,
  logs: false,
  metrics: false,
  usage: false,
  rollback: false,
  pause: false,
  domains: false,
  secrets: false,
  discover: true,
  server: false,
  provider_details: false,
  provider_deploys: false,
  provider_variables: false,
  provider_operations: false,
  upload: false,
  new_project: false,
  new_from_folder: false,
  new_database: false,
  new_bucket: false,
  new_email_domain: false,
  new_from_image: false,
  bills: false,
  browser_sign_in: false,
  delete: false,
  plan_deploy: false,
  containers: false,
  cron: false,
  project_database: false,
  dns_zones: false,
};

/** What `GET /providers` answers, for three providers the engine has adapters for. */
const providers: ProviderAbilities = {
  items: [
    ['cloudflare', 'Cloudflare', 'account ID'],
    ['fly', 'Fly.io', 'organization slug'],
    ['railway', 'Railway', 'workspace ID'],
  ].map(([provider, name, account_label]) => ({
    provider: provider as ProviderAbilities['items'][number]['provider'],
    name: name ?? '',
    account_label: account_label ?? '',
    links: {
      signup: `https://${provider}.example.com/sign-up`,
      tokens: `https://${provider}.example.com/tokens`,
    },
    capabilities: none,
  })),
};

/** Provider sign-in is off in Hoista's builds for now; these tests turn it on for Railway. */
const railwaySignsIn = {
  items: providers.items.map((entry) =>
    entry.provider === 'railway'
      ? { ...entry, capabilities: { ...entry.capabilities, browser_sign_in: true } }
      : entry,
  ),
};

const saved: DaemonAccount = {
  id: 'acc-1',
  provider: 'cloudflare',
  label: 'Ada',
  external_id: 'cf-123',
  scopes: ['workers:write'],
  status: 'connected',
  connected_at: '2026-11-02T00:00:00Z',
  last_discovery: {
    observed_at: '2026-11-02T00:01:00Z',
    complete: true,
    observed_resources: 3,
    warnings: [],
  },
  credential: null,
  in_1password: false,
};

function daemonWith(
  choices: { external_id: string; name: string }[],
  abilities: typeof providers = providers,
) {
  const asked: EngineRequest[] = [];
  const daemon: Engine = {
    request: (request) => {
      // What each provider's adapter can do; not one of the calls being checked.
      if (request.path === '/providers') return Promise.resolve({ ok: true, data: abilities });
      asked.push(request);
      if (request.path === '/accounts/lookup') return Promise.resolve({ ok: true, data: choices });
      if (request.method === 'POST' && request.path === '/accounts') {
        return Promise.resolve({ ok: true, data: saved });
      }
      if (request.path === '/accounts') {
        return Promise.resolve({ ok: true, data: { items: [saved], next_cursor: null } });
      }
      if (request.path === '/projects/summaries') {
        return Promise.resolve({ ok: true, data: { items: [], next_cursor: null } });
      }
      return Promise.resolve({ ok: false, error: { code: 'not_found', message: request.path } });
    },
    subscribe: () => () => undefined,
  };
  return { adapter: createAdapter(daemon), asked };
}

describe('accounts from the daemon', () => {
  it('connects straight away when the token reaches one account', async () => {
    const { adapter, asked } = daemonWith([{ external_id: 'cf-123', name: 'Ada' }]);
    const result = unwrap<ConnectResult>(
      await adapter.request({
        method: 'POST',
        path: '/accounts',
        body: { service: 'cloudflare', token: 'secret' },
      }),
    );
    expect(
      asked.find((request) => request.method === 'POST' && request.path === '/accounts')?.body,
    ).toEqual({ provider: 'cloudflare', label: 'Ada', external_id: 'cf-123', token: 'secret' });
    expect(result.account).toMatchObject({ service: 'cloudflare', status: 'ok', token_hint: '' });
    expect(result.checks.map((check) => check.label)).toEqual([
      'Token works',
      'Found 3 existing resources',
    ]);
  });

  it('asks the person to choose instead of guessing between accounts', async () => {
    const { adapter } = daemonWith([
      { external_id: 'a', name: 'Personal' },
      { external_id: 'b', name: 'Work' },
    ]);
    const result = await adapter.request({
      method: 'POST',
      path: '/accounts',
      body: { service: 'cloudflare', token: 'secret' },
    });
    expect(result.ok ? null : result.error.code).toBe('account.choice_required');
  });

  it('refuses services the engine cannot connect yet, before sending the token', async () => {
    const { adapter, asked } = daemonWith([]);
    const result = await adapter.request({
      method: 'POST',
      path: '/accounts',
      body: { service: 'netlify', token: 'secret' },
    });
    expect(result.ok ? null : result.error.code).toBe('account.unsupported');
    expect(asked).toEqual([]);
  });

  it('replaces a token by reconnecting the same account', async () => {
    const { adapter, asked } = daemonWith([]);
    await adapter.request({ method: 'PUT', path: '/accounts/acc-1/token', body: { token: 'new' } });
    expect(asked.at(-1)?.body).toEqual({
      provider: 'cloudflare',
      label: 'Ada',
      external_id: 'cf-123',
      token: 'new',
    });
  });

  it('marks services without an engine adapter as coming soon', async () => {
    const { adapter } = daemonWith([]);
    const list = unwrap<Page<CatalogueEntry>>(
      await adapter.request({ method: 'GET', path: '/catalogue' }),
    );
    expect(list.items.find((entry) => entry.id === 'cloudflare')?.availability).toBe('available');
    expect(list.items.find((entry) => entry.service === 'netlify')?.availability).toBe(
      'coming_soon',
    );
  });

  it('describes a broken token without inventing when it broke', () => {
    const account = toAccount({ ...saved, status: 'expired' }, []);
    expect(account.status).toBe('expired');
    expect(account.problem?.since).toBeNull();
    expect(checksFor({ ...saved, last_discovery: null })).toHaveLength(1);
  });
});

describe('connecting on the provider’s page', () => {
  it('opens the provider’s page, then finishes once the person clicks Allow', async () => {
    const asked: EngineRequest[] = [];
    const opened: string[] = [];
    const railway: DaemonAccount = { ...saved, provider: 'railway', label: 'Sam’s workspace' };
    const daemon: Engine = {
      request: (request) => {
        if (request.path === '/providers')
          return Promise.resolve({ ok: true, data: railwaySignsIn });
        asked.push(request);
        if (request.path === '/accounts/sign-in') {
          return Promise.resolve({
            ok: true,
            data: { id: 's1', url: 'https://backboard.railway.com/oauth/auth?x=1' },
          });
        }
        if (request.path === '/accounts/sign-in/s1/finish') {
          return Promise.resolve({ ok: true, data: railway });
        }
        return Promise.resolve({ ok: false, error: { code: 'not_found', message: request.path } });
      },
      subscribe: () => () => undefined,
    };
    const adapter = createAdapter(daemon, {
      openPage: (url) => {
        opened.push(url);
        return Promise.resolve();
      },
    });
    const result = unwrap<ConnectResult>(
      await adapter.request({
        method: 'POST',
        path: '/accounts/sign-in',
        body: { service: 'railway' },
      }),
    );
    expect(opened).toEqual(['https://backboard.railway.com/oauth/auth?x=1']);
    expect(asked.map((request) => request.path)).toEqual([
      '/accounts/sign-in',
      '/accounts/sign-in/s1/finish',
    ]);
    expect(asked[0]?.body).toEqual({ provider: 'railway' });
    expect(result.account).toMatchObject({ service: 'railway', label: 'Sam’s workspace' });
    expect(result.checks[0]?.label).toBe('Signed in on Railway');
  });

  it('offers it only for providers the engine can sign in to', async () => {
    const { adapter, asked } = daemonWith([], railwaySignsIn);
    const fly = await adapter.request({
      method: 'POST',
      path: '/accounts/sign-in',
      body: { service: 'fly' },
    });
    expect(fly.ok ? null : fly.error.code).toBe('account.sign_in_unavailable');
    expect(asked).toEqual([]);
    const guide = (path: string) => adapter.request({ method: 'GET', path });
    const railway = await guide('/catalogue/railway/connect');
    const cloudflare = await guide('/catalogue/cloudflare/connect');
    expect(railway.ok && (railway.data as { sign_in: unknown }).sign_in).toBeTruthy();
    expect(cloudflare.ok && (cloudflare.data as { sign_in: unknown }).sign_in).toBeNull();
  });
});

describe('scanNote', () => {
  const scan = {
    observed_at: '2026-11-02T00:01:00Z',
    complete: true,
    observed_resources: 3,
    warnings: [],
  };
  it('is empty when the scan read everything', () => {
    expect(scanNote({ ...scan, complete: true, observed_resources: 3, warnings: [] })).toBeNull();
  });
  it('names what a scan couldn’t read, in the provider’s words', () => {
    const note = scanNote({
      ...scan,
      complete: false,
      observed_resources: 1,
      warnings: [
        { resource_type: 'pages', code: 'provider.rejected', message: '' },
        { resource_type: 'r2:eu', code: 'provider.token_scope', message: '' },
        { resource_type: 'r2:us', code: 'provider.token_scope', message: '' },
      ],
    });
    expect(note).toContain('Pages projects, R2 buckets');
  });
  it('says when a scan didn’t finish', () => {
    expect(scanNote({ ...scan, complete: false, observed_resources: null, warnings: [] })).toMatch(
      /couldn’t finish/,
    );
  });
});
