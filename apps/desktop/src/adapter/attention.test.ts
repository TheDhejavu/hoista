import { describe, expect, it } from 'vitest';
import type { ProjectOverview, Deploy, Operation } from '@engine-api';
import { noticeItem, unclearItem } from './attention';

const overview = {
  project: { id: 'shop', name: 'shop' },
  resources: [{ id: 'api', name: 'shop-api' }],
} as unknown as ProjectOverview;
const projects = new Map([['shop', overview]]);

describe('needs attention from the daemon', () => {
  it('names missing keys on a failed deploy', () => {
    const deploy = {
      id: 'd1',
      failure: { message: 'Set it.', missing_secrets: ['RESEND_API_KEY'] },
    } as unknown as Deploy;
    const [item] = noticeItem(
      { kind: 'deployment_failed', project_id: 'shop', deploy_id: 'd1' },
      projects,
      new Map([['d1', deploy]]),
    );
    expect(item).toMatchObject({
      kind: 'failure',
      tone: 'bad',
      title: 'shop can’t deploy',
      body: 'Missing RESEND_API_KEY',
    });
  });

  it('words an idle resource by its own name', () => {
    const [item] = noticeItem(
      {
        kind: 'idle_resource',
        project_id: 'shop',
        resource_id: 'api',
        since: '2026-10-10',
        through: '2026-11-01',
        days: 22,
      },
      projects,
      new Map(),
    );
    expect(item?.title).toBe('shop-api had no requests for 22 days');
  });

  it('asks about a change Hoista didn’t hear back about, and nothing else', () => {
    const operation: Operation = {
      id: 'op-1',
      provider: 'railway',
      account: 'ws',
      kind: 'create',
      target: 'service web next to p1/s1',
      state: 'unclear',
      repeats: true,
      started_at: '2026-11-04T10:00:00.000Z',
      finished_at: '2026-11-04T10:00:05.000Z',
      message: 'The provider could not be reached.',
    };
    const [item] = unclearItem(operation);
    expect(item).toMatchObject({
      kind: 'unclear',
      title: 'Did this happen at Railway?',
      body: "Service web next to p1/s1 · Hoista didn't hear back",
      operation: { id: 'op-1', service: 'railway' },
    });
    expect(unclearItem({ ...operation, state: 'done' })).toEqual([]);
    expect(unclearItem({ ...operation, state: 'did_not_happen' })).toEqual([]);
  });
});
