import { describe, expect, it } from 'vitest';
import type { ProjectOverview } from '@engine-api';
import { Router } from '@shared/router';
import type { Call } from './call';
import type { ProviderReads } from './provider';
import { addOverviewRoutes, toProjectDetail } from './overview';

const base: ProjectOverview = {
  project: {
    id: 'shop',
    name: 'shop',
    source: { kind: 'github', repository: 'sam/shop', commit: '0123456789abcdef' },
    mode: 'managed',
    created_at: '2026-11-01T00:00:00Z',
    environment_id: 'env_default',
  },
  status: 'live',
  resources: [
    {
      id: 'db',
      project_id: 'shop',
      kind: 'database',
      provider: 'neon',
      account_id: 'neon',
      provider_ref: 'project/abc',
      name: 'shop-db',
      region: 'aws-eu-central-1',
      status: 'sleeping',
      mode: 'managed',
      url: null,
      group: null,
    },
  ],
  last_deploy: null,
  source_git: null,
  stats: {
    traffic: [],
    requests: null,
    error_rate: null,
    p99: 412,
    last_observed_day: '2026-11-02',
  },
};

describe('project overview from the daemon', () => {
  it('passes measured values through and leaves unmeasured ones empty', () => {
    const detail = toProjectDetail(base, new Map());
    expect(detail.project.source).toEqual({ kind: 'github', ref: 'sam/shop@0123456' });
    expect(detail.stats).toMatchObject({
      p99_ms: 412,
      requests: null,
      requests_change: null,
      errors_24h: null,
      top_error: null,
    });
    expect(detail.resources[0]).toMatchObject({ product: 'Postgres', usage: null });
  });
});

describe('deleting a project', () => {
  function routes(
    preview: { blockers: string[] },
    resources: { id: string; name: string; provider: string; mode: string }[] = [],
  ) {
    const calls: { method: string; path: string; body?: unknown }[] = [];
    let gone = false;
    const call = ((method: string, path: string, body?: unknown) => {
      calls.push({ method, path, body });
      if (method === 'POST' && path.endsWith('/provider/actions')) gone = true;
      if (method === 'GET' && path === '/projects/vite-react-test') {
        return gone
          ? Promise.reject(new Error('not found'))
          : Promise.resolve({ project: { mode: 'managed' }, resources });
      }
      if (path === '/providers') {
        return Promise.resolve({
          items: [{ provider: 'modal', capabilities: { delete: true } }],
        });
      }
      return Promise.resolve(
        path.endsWith('/deletion-preview')
          ? { ...preview, confirmation_name: 'vite-react-test', fingerprint: 'f1' }
          : { removed: true },
      );
    }) as Call;
    const router = new Router();
    addOverviewRoutes(router, call, {} as ProviderReads);
    return { router, calls };
  }

  it('sends the reviewed name and fingerprint the engine asks for', async () => {
    const { router, calls } = routes({ blockers: [] });
    await router.dispatch(
      { method: 'DELETE', path: '/projects/vite-react-test', body: { backup: true } },
      () => undefined,
    );
    expect(calls.at(-1)).toEqual({
      method: 'DELETE',
      path: '/projects/vite-react-test',
      body: { backup_databases: true, confirmation_name: 'vite-react-test', fingerprint: 'f1' },
    });
  });

  it('deletes what runs on a provider there, and the project goes with it', async () => {
    const { router, calls } = routes({ blockers: ['On its provider.'] }, [
      { id: 'r1', name: 'sentiment', provider: 'modal', mode: 'watching' },
    ]);
    await router.dispatch(
      { method: 'DELETE', path: '/projects/vite-react-test', body: { backup: true } },
      () => undefined,
    );
    expect(calls).toContainEqual({
      method: 'POST',
      path: '/resources/r1/provider/actions',
      body: { action: 'remove_service', confirmation_name: 'sentiment' },
    });
    expect(calls.some((entry) => entry.method === 'DELETE')).toBe(false);
  });

  it('says why when something stops the delete', async () => {
    const { router, calls } = routes({ blockers: ['A deploy is running.'] });
    const result = await router.dispatch(
      { method: 'DELETE', path: '/projects/vite-react-test', body: { backup: true } },
      () => undefined,
    );
    expect(JSON.stringify(result)).toContain('A deploy is running.');
    expect(calls.some((entry) => entry.method === 'DELETE')).toBe(false);
  });
});
