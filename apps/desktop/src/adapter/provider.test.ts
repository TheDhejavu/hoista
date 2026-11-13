import { describe, expect, it } from 'vitest';
import type { ProjectOverview, ProviderDetails, Resource } from '@engine-api';
import { Router } from '@shared/router';
import type { Call } from './call';
import { addProviderRoutes, providerDeploys, type ProviderReads } from './provider';
import { toFound } from './imports';

const resource = (id: string, project: string | null, kind: Resource['kind'], name: string) =>
  ({
    id,
    project_id: project,
    kind,
    provider: 'railway',
    account_id: 'rw',
    provider_ref: `proj/${id}`,
    name,
    region: null,
    status: 'live',
    mode: 'watching',
    url: null,
  }) as Resource;

const details: ProviderDetails = {
  deployments: [
    {
      id: 'd3',
      status: 'BUILDING',
      state: 'in_progress',
      created_at: '2026-11-03T10:00:00Z',
      finished_at: null,
      can_roll_back: null,
      url: null,
      commit: null,
      message: null,
      branch: null,
      repo: null,
      author: null,
      provider_page: null,
    },
    {
      id: 'd2',
      status: 'SUCCESS',
      state: 'succeeded',
      created_at: '2026-11-02T10:00:00Z',
      finished_at: null,
      can_roll_back: null,
      url: 'https://api.sam.dev',
      commit: 'abc1234567',
      message: 'Fix login',
      branch: 'main',
      repo: 'sam/api',
      author: 'Sam Rivera',
      provider_page: null,
    },
    {
      id: 'd1b',
      status: 'REMOVED',
      state: 'stopped',
      created_at: '2026-11-01T12:00:00Z',
      finished_at: null,
      can_roll_back: true,
      url: null,
      commit: null,
      message: null,
      branch: null,
      repo: null,
      author: null,
      provider_page: null,
    },
    {
      id: 'd1a',
      status: 'REMOVED',
      state: 'stopped',
      created_at: '2026-11-01T11:00:00Z',
      finished_at: null,
      can_roll_back: false,
      url: null,
      commit: null,
      message: null,
      branch: null,
      repo: null,
      author: null,
      provider_page: null,
    },
    {
      id: 'd1',
      status: 'SUCCESS',
      state: 'succeeded',
      created_at: '2026-11-01T10:00:00Z',
      finished_at: null,
      can_roll_back: null,
      url: null,
      commit: null,
      message: null,
      branch: null,
      repo: null,
      author: null,
      provider_page: null,
    },
  ],
  variable_names: ['DATABASE_URL'],
  domains: [],
  references: [],
  sleeps: null,
  config: null,
  volumes: [],
  tcp_proxies: [],
  shared_variable_names: null,
  private_domain: null,
  addresses: [],
};

describe('provider deployments on the Deploys tab', () => {
  it('shows the provider history with the newest success current and earlier ones to roll back to', () => {
    const deploys = providerDeploys(
      'project',
      resource('svc', 'project', 'service', 'api'),
      details,
    );
    expect(deploys.map((deploy) => [deploy.version, deploy.status, deploy.current])).toEqual([
      ['d3', 'running', false],
      ['abc1234', 'live', true],
      ['d1b', 'live', false],
      ['d1a', 'cancelled', false],
      ['d1', 'live', false],
    ]);
    // Replaced, and the provider can't go back to it: said plainly, without guessing whether it ever served.
    expect(deploys[3]?.failure?.message).toBe('Replaced by a newer deploy on Railway.');
    expect(deploys[2]?.failure).toBeNull();
    // The commit's author when the provider says, otherwise the provider.
    expect(deploys[1]?.started_by).toEqual({ kind: 'person', name: 'Sam Rivera' });
    expect(deploys[0]?.started_by).toEqual({ kind: 'system', name: 'Railway' });
    expect(deploys[1]?.source.message).toBe('Fix login');
  });
});

describe('found resources in a project a project already holds', () => {
  it('offers to add them to that project and to combine split projects', () => {
    const project = (id: string, resources: Resource[]) =>
      ({ project: { id, name: id, mode: 'watching' }, resources }) as unknown as ProjectOverview;
    const found = toFound(
      [resource('cache', null, 'service', 'Redis')],
      [],
      [
        project('api', [resource('svc', 'api', 'service', 'api')]),
        project('postgres', [resource('pg', 'postgres', 'database', 'Postgres')]),
      ],
    );
    expect(found.resources[0]?.join).toEqual({ project_id: 'api', project_name: 'api' });
    expect(found.combine).toEqual([
      {
        from: { id: 'postgres', name: 'postgres' },
        into: { id: 'api', name: 'api' },
        provider: 'railway',
        reason: 'Same Railway project',
      },
    ]);
  });
});

describe('the Domains tab of a project with watched and deployed services', () => {
  it('lists every service’s own address, not only what the watched providers report', async () => {
    const vercel = { ...resource('web', 'p', 'web', 'dashboard'), url: 'https://dash.vercel.app' };
    const fly = {
      ...resource('api', 'p', 'service', 'api'),
      mode: 'managed',
      url: 'https://api.fly.dev',
    } as Resource;
    const worker = {
      ...resource('edge', 'p', 'web', 'edge'),
      url: 'https://edge.sam.workers.dev',
    };
    const call = ((_method: string, path: string) =>
      Promise.resolve(
        path.endsWith('/domains')
          ? { items: [], cursor: null }
          : ({ resources: [vercel, fly, worker] } as unknown as ProjectOverview),
      )) as Call;
    const reads = {
      forProject: () =>
        Promise.resolve([
          {
            resource: vercel,
            details: {
              ...details,
              domains: [
                { name: 'dash.vercel.app', custom: false, state: 'working', dns: [], records: [] },
              ],
            } as unknown as ProviderDetails,
          },
        ]),
    } as unknown as ProviderReads;
    const router = new Router();
    addProviderRoutes(router, call, reads);
    const result = (await router.dispatch(
      { method: 'GET', path: '/projects/p/domains', body: null },
      () => undefined,
    )) as { ok: true; data: { items: { name: string }[] } };
    const names = JSON.stringify(result);
    expect(names).toContain('dash.vercel.app');
    expect(names).toContain('api.fly.dev');
    expect(names).toContain('edge.sam.workers.dev');
    expect(names.match(/dash\.vercel\.app/g)?.length).toBe(1);
  });
});
