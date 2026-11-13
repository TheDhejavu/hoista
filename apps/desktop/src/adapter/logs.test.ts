import { describe, expect, it } from 'vitest';
import { Router, RouteError } from '@shared/router';
import type { Call } from './call';
import { addLogRoutes } from './logs';

const overview = {
  project: { id: 'notes' },
  resources: [
    { id: 'api', kind: 'service', provider: 'railway', name: 'api' },
    { id: 'db', kind: 'database', provider: 'neon', name: 'db' },
  ],
};

function routes(history: (path: string) => unknown): Router {
  const router = new Router();
  const call = ((_method: string, path: string) =>
    Promise.resolve().then(() => (path === '/projects/notes' ? overview : history(path)))) as Call;
  addLogRoutes(router, call);
  return router;
}

describe('project logs from daemon history', () => {
  it('reads resources that run code and merges their lines oldest first', async () => {
    const paths: string[] = [];
    const router = routes((path) => {
      paths.push(path);
      return {
        items: [
          {
            timestamp: '2026-11-03T10:00:02Z',
            observed_at: 'x',
            level: 'error',
            source: 'deployment',
            message: 'boom',
          },
          {
            timestamp: '2026-11-03T10:00:01Z',
            observed_at: 'x',
            level: 'debug',
            source: 'deployment',
            message: 'boot',
          },
        ],
        next_cursor: null,
        query_sampling_level: null,
        coverage_complete: false,
      };
    });
    const page = (await router.dispatch(
      { method: 'GET', path: '/projects/notes/logs?since_minutes=60&q=bo' },
      () => undefined,
    )) as { ok: true; data: { items: { message: string; level: string }[] } };
    const history = paths.filter((path) => path.includes('/logs/history'));
    expect(history).toHaveLength(1);
    expect(history[0]).toContain('/projects/notes/logs/history?resource=api');
    expect(history[0]).toContain('query=bo');
    // Railway's served requests are read for the same window.
    expect(paths).toContain('/resources/api/provider');
    expect(page.data.items.map((line) => [line.message, line.level])).toEqual([
      ['boot', 'info'],
      ['boom', 'error'],
    ]);
  });

  it('says when no resource can be read instead of showing an empty table', async () => {
    const router = routes(() => {
      throw new RouteError('provider.not_supported', 'Not supported by this provider.');
    });
    const result = await router.dispatch(
      { method: 'GET', path: '/projects/notes/logs' },
      () => undefined,
    );
    expect(result).toMatchObject({
      ok: false,
      error: { message: 'Not supported by this provider.' },
    });
  });
});

describe('request lines', () => {
  it('reads a Railway request as a line with its facts', async () => {
    const { toRequestLine } = await import('./logs');
    const line = toRequestLine(
      { id: 'p1', name: 'api', kind: 'service' } as Parameters<typeof toRequestLine>[0],
      {
        timestamp: '2026-11-03T10:00:00Z',
        method: 'POST',
        path: '/upload',
        httpStatus: 502,
        totalDuration: 1830.4,
        edgeRegion: 'europe-west4',
        txBytes: 2_400_000,
      },
    );
    expect(line.what).toBe('POST /upload');
    expect(line.message).toBe('502 · 1830ms');
    expect(line.level).toBe('error');
    expect(line.source).toBe('request');
    expect(line.facts).toContainEqual({ label: 'Sent', value: '2.4 MB' });
  });
});
