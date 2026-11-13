import { describe, expect, it } from 'vitest';
import type { Deploy } from '@engine-api';
import { currentIds, toDeploy } from './deploys';

function attempt(change: Partial<Deploy>): Deploy {
  return {
    id: 'd1',
    project_id: 'shop',
    version: 1,
    release_version: 1,
    source: { kind: 'folder', path: '/code/shop' },
    started_by: 'person',
    started_by_name: null,
    status: 'live',
    steps: [
      { name: 'detect', status: 'succeeded', message: 'Vite project' },
      { name: 'automatic_rollback', status: 'failed', message: null },
    ],
    url: 'https://shop.example.dev',
    started_at: '2026-11-02T10:00:00Z',
    finished_at: '2026-11-02T10:01:00Z',
    duration_ms: 60000,
    failure: null,
    title: 'Fix checkout',
    ...change,
  } as Deploy;
}

describe('deploys from the daemon', () => {
  it('shows release numbers, pipeline steps and the title', () => {
    const deploy = toDeploy(attempt({}), true, new Map());
    expect(deploy.version).toBe('v1');
    expect(deploy.source).toEqual({ kind: 'folder', ref: '/code/shop', message: 'Fix checkout' });
    expect(deploy.steps).toEqual([
      { name: 'detect', status: 'done', detail: 'Vite project', duration_ms: null },
    ]);
    expect(deploy.started_by).toEqual({ kind: 'person', name: 'You' });
  });

  it('labels failed attempts by attempt and keeps the failing step', () => {
    const deploy = toDeploy(
      attempt({
        version: 7,
        release_version: null,
        status: 'failed',
        failure: {
          code: 'deploy.secret_missing',
          message: 'Set RESEND_API_KEY.',
          steps: ['secrets'],
          missing_secrets: ['RESEND_API_KEY'],
          release_started: false,
          compensation_failed: false,
          provider_response: null,
        },
      }),
      false,
      new Map(),
    );
    expect(deploy.version).toBe('attempt 7');
    expect(deploy.failure).toEqual({
      step: 'secrets',
      message: 'Set RESEND_API_KEY.',
      fix: null,
      missing_secrets: ['RESEND_API_KEY'],
    });
  });

  it('shows a stopped deploy as cancelled, not failed', () => {
    const deploy = toDeploy(
      attempt({
        status: 'failed',
        release_version: null,
        failure: {
          code: 'deploy.cancelled',
          message: 'Stopped.',
          steps: [],
          missing_secrets: [],
          release_started: false,
          compensation_failed: false,
          provider_response: null,
        },
      }),
      false,
      new Map(),
    );
    expect(deploy.status).toBe('cancelled');
    expect(deploy.failure).toBeNull();
  });

  it('marks each project’s newest live attempt as current', () => {
    const list = [
      attempt({ id: 'a3', status: 'failed' }),
      attempt({ id: 'a2', status: 'live' }),
      attempt({ id: 'a1', status: 'live' }),
      attempt({ id: 'b1', project_id: 'blog', status: 'live' }),
    ];
    expect([...currentIds(list)]).toEqual(['a2', 'b1']);
  });
});
