import { describe, expect, it } from 'vitest';
import type { Deploy } from '@shared/api';
import {
  canRollBackTo,
  changeTitle,
  changesUndoneBy,
  failingDeploys,
  previousVersion,
} from './describe';

/** A deploy of moodboard, `hours` ago. */
const past = (
  version: string,
  status: Deploy['status'],
  hours: number,
  message: string,
  current = false,
): Deploy => ({
  id: `dep-moodboard-${version}-${String(hours)}`,
  project_id: 'moodboard',
  version,
  source: { kind: 'commit', ref: 'main', message },
  started_by: { kind: 'person', name: 'You' },
  status,
  steps: [],
  url: status === 'live' ? 'https://moodboard.example.com' : null,
  created_at: new Date(Date.UTC(2026, 10, 4) - hours * 3_600_000).toISOString(),
  duration_ms: 40_000,
  current,
  failure: null,
});

/** Newest first: v4 live now, v3 before it, a failed and a stopped try, then v2. */
const moodboard: Deploy[] = [
  past('v4', 'live', 2, 'Add share button', true),
  past('v3', 'live', 24, 'Fix upload size limit'),
  past('—', 'failed', 48, 'Try new layout'),
  past('—', 'cancelled', 49, 'Try dark header'),
  past('v2', 'live', 50, 'Add uploads'),
];

describe('roll back helpers', () => {
  it('picks the newest live version before the current one', () => {
    expect(previousVersion(moodboard)?.version).toBe('v3');
  });

  it('skips failed deploys when listing what a roll back undoes', () => {
    const v2 = moodboard.find((deploy) => deploy.version === 'v2');
    if (!v2) throw new Error('fixture missing v2');

    expect(changesUndoneBy(moodboard, v2).map((deploy) => deploy.version)).toEqual(['v4', 'v3']);
  });
});

describe('changeTitle', () => {
  const base = moodboard[0];
  if (!base) throw new Error('fixture missing');
  const titled = (message: string | null): string =>
    changeTitle({ ...base, source: { ...base.source, message } });

  it('keeps only the commit subject and drops markdown links', () => {
    expect(
      titled(
        'Bump express (#418)\n\nBumps the node-dependencies group with 1 update: [express](https://github.com/expressjs/express).',
      ),
    ).toBe('Bump express (#418)');
    expect(titled('Use [Railpack](https://railpack.com) to build')).toBe('Use Railpack to build');
  });

  it('falls back to the source when there is no message', () => {
    expect(titled(null)).toBe(base.source.ref);
  });
});

const deploy = (
  id: string,
  resource: string | undefined,
  status: Deploy['status'],
  at: string,
  current = false,
): Deploy =>
  ({
    id,
    resource_id: resource,
    status,
    created_at: at,
    current,
  }) as Deploy;

describe('deploys per resource', () => {
  it('finds the resource whose own newest deploy failed, not another service’s', () => {
    const deploys = [
      deploy('w2', 'web', 'live', '2026-11-04T10:00:00Z', true),
      deploy('a2', 'api', 'failed', '2026-11-04T09:00:00Z'),
      deploy('a1', 'api', 'live', '2026-11-03T09:00:00Z', true),
      deploy('w1', 'web', 'live', '2026-11-03T08:00:00Z'),
    ];
    expect(failingDeploys(deploys)?.map((item) => item.id)).toEqual(['a2', 'a1']);
    expect(failingDeploys(deploys.filter((item) => item.id !== 'a2'))).toBeNull();
  });

  it('treats a managed project as one whole', () => {
    expect(
      failingDeploys([deploy('v3', undefined, 'failed', '2026-11-04T10:00:00Z')])?.length,
    ).toBe(1);
  });

  it('rolls back only where the provider says it can', () => {
    expect(canRollBackTo({ ...deploy('a', 'api', 'live', 'x'), can_roll_back: false })).toBe(false);
    expect(canRollBackTo(deploy('a', 'api', 'live', 'x'))).toBe(true);
    expect(canRollBackTo(deploy('a', 'api', 'live', 'x', true))).toBe(false);
  });
});
