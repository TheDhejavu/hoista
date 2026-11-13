import { describe, expect, it } from 'vitest';
import type { ProjectOverview, ProjectTotals, Deploy, Page, Resource } from '@engine-api';
import type { ProjectList, EngineRequest, EngineResult, Profile } from '@shared/api';
import { unwrap, type Engine } from '@shared/engine';
import { toProjectListItem } from './projects';
import { createAdapter } from '.';
import { initialsOf } from './profile';

/** A daemon stand-in: answers from a table and records what it was asked. */
function fakeDaemon(answers: Record<string, unknown>) {
  const asked: EngineRequest[] = [];
  const daemon: Engine = {
    request: (request): Promise<EngineResult> => {
      asked.push(request);
      const key = `${request.method} ${request.path}`;
      if (key in answers) {
        const answer = answers[key];
        return Promise.resolve(
          answer instanceof Error
            ? { ok: false, error: { code: answer.message, message: 'From the daemon.' } }
            : { ok: true, data: answer },
        );
      }
      return Promise.resolve({ ok: false, error: { code: 'not_found', message: key } });
    },
    subscribe: () => () => undefined,
  };
  return { daemon, asked };
}

const resource: Resource = {
  id: 'site-web',
  project_id: 'site',
  kind: 'web',
  provider: 'cloudflare',
  account_id: 'cf',
  provider_ref: 'worker:site',
  name: 'site',
  region: null,
  status: 'live',
  mode: 'managed',
  url: 'https://site.example.workers.dev',
  group: null,
};

function overview(change: Partial<ProjectOverview> = {}): ProjectOverview {
  return {
    project: {
      id: 'site',
      name: 'site',
      source: { kind: 'unlinked' },
      mode: 'managed',
      created_at: '2026-11-01T00:00:00Z',
      environment_id: 'env_default',
    },
    status: 'live',
    resources: [resource],
    last_deploy: null,
    source_git: null,
    stats: {
      traffic: [1, 2, 3, 4, 5, 6, 7].map((requests, i) => ({
        day: `2026-11-0${String(i + 1)}`,
        requests,
        errors: 0,
      })),
      requests: 28,
      error_rate: 0,
      p99: null,
      last_observed_day: null,
    },
    ...change,
  };
}

describe('Projects list from the daemon', () => {
  it('names who deployed', () => {
    const deploy = {
      status: 'failed',
      started_by: 'agent-1',
      started_by_name: null,
      started_at: '2026-11-02T10:00:00Z',
      finished_at: '2026-11-02T10:01:00Z',
    } as unknown as Deploy;
    const item = toProjectListItem(
      overview({ last_deploy: deploy }),
      new Map([['agent-1', 'Codex']]),
    );
    expect(item.url).toBe('https://site.example.workers.dev');
    expect(item.runs_on).toEqual(['Cloudflare']);
    expect(item.sparkline).toEqual([1, 2, 3, 4, 5, 6, 7]);
    expect(item.last_deploy).toEqual({
      at: '2026-11-02T10:01:00Z',
      status: 'failed',
      by: { kind: 'agent', name: 'Codex' },
    });
  });

  it('drops the sparkline when any day is missing', () => {
    const base = overview();
    const gap = overview({
      stats: {
        ...base.stats,
        traffic: base.stats.traffic.map((day, i) => (i === 3 ? { ...day, requests: null } : day)),
      },
    });
    expect(toProjectListItem(gap, new Map()).sparkline).toBeNull();
  });

  it('serves GET /projects from summaries, totals and agents', async () => {
    const totals: ProjectTotals = {
      projects: 1,
      live: 1,
      failing: 0,
      providers: 1,
    };
    // Two pages: the list follows the cursor and holds every project.
    const summaries: Page<ProjectOverview> = { items: [overview()], next_cursor: 'next' };
    const second: Page<ProjectOverview> = {
      items: [
        { ...overview(), project: { ...overview().project, id: 'project-2', name: 'second' } },
      ],
      next_cursor: null,
    };
    const { daemon, asked } = fakeDaemon({
      'GET /projects/summaries?q=si&sort=name': summaries,
      'GET /projects/summaries?q=si&sort=name&cursor=next': second,
      'GET /projects/totals': totals,
      'GET /agents': { items: [], next_cursor: null },
    });
    const adapter = createAdapter(daemon);
    const list = unwrap<ProjectList>(
      await adapter.request({ method: 'GET', path: '/projects?q=si&sort=name' }),
    );
    expect(list.cursor).toBeNull();
    expect(list.items.map((item) => item.name)).toHaveLength(2);
    expect(list.totals).toEqual({
      projects: 1,
      live: 1,
      failing: 0,
      providers: 1,
    });
    expect(asked.map((request) => request.path)).toContain('/projects/summaries?q=si&sort=name');
  });
});

describe('profile from the daemon', () => {
  it('reads a missing profile as null, the first-run state', async () => {
    const { daemon } = fakeDaemon({ 'GET /profile': new Error('profile.not_found') });
    const result = await createAdapter(daemon).request({ method: 'GET', path: '/profile' });
    expect(result).toEqual({ ok: true, data: null });
  });

  it('saves a name with initials worked out, the way the daemon expects', async () => {
    const { daemon, asked } = fakeDaemon({
      'PUT /profile': { name: 'Ada Lovelace', initials: 'AL' },
    });
    const saved = unwrap<Profile>(
      await createAdapter(daemon).request({
        method: 'PUT',
        path: '/profile',
        body: { name: ' Ada Lovelace ' },
      }),
    );
    expect(asked[0]?.body).toEqual({ name: 'Ada Lovelace', initials: 'AL' });
    expect(saved).toEqual({ name: 'Ada Lovelace', initials: 'AL' });
  });

  it('works out initials', () => {
    expect(initialsOf('Ada')).toBe('A');
    expect(initialsOf('ada king lovelace')).toBe('AL');
  });
});

describe('everything else', () => {
  it('passes through unchanged', async () => {
    const { daemon, asked } = fakeDaemon({ 'GET /doctor': { checks: [] } });
    const result = await createAdapter(daemon).request({ method: 'GET', path: '/doctor' });
    expect(result).toEqual({ ok: true, data: { checks: [] } });
    expect(asked).toHaveLength(1);
  });
});
