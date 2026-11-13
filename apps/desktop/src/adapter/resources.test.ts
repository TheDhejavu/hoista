import { describe, expect, it } from 'vitest';
import type { ResourceDetail } from '@engine-api';
import { computeTime, toResourceDetail } from './resources';

const detail = {
  resource: {
    id: 'svc',
    project_id: 'notes',
    kind: 'service',
    provider: 'railway',
    account_id: 'rw',
    provider_ref: 'proj-1/svc-1',
    name: 'notes-api',
    region: null,
    status: 'unknown',
    mode: 'watching',
    url: null,
  },
  project: null,
  account: { label: 'Ada', external_id: 'ws-1' },
  dependencies: [],
  used_by: [],
  domains: [],
  latest_usage: null,
  latest_recorded_release: null,
  relationships_complete: false,
} as unknown as ResourceDetail;

describe('resource page from the daemon', () => {
  it('shows recorded facts only and links to the provider console', () => {
    const page = toResourceDetail(detail);
    expect(page.resource.name).toBe('notes-api');
    expect(page.summary).toBe('Railway · Service · Ada');
    expect(page.stats).toEqual([]);
    expect(page.events).toEqual([]);
    expect(page.status_note).toMatch(/does not report/);
    expect(page.settings).toContainEqual({ label: 'Hoista', value: 'Watching (read-only)' });
    expect(page.console_url).toBe('https://railway.com/project/proj-1/service/svc-1');
  });

  it('shows measured Railway CPU and disk, and no cost', () => {
    const page = toResourceDetail({
      ...detail,
      latest_usage: {
        resource_id: 'svc',
        day: '2026-11-02',
        requests: null,
        errors: null,
        compute_seconds: 60_000,
        storage_bytes: 2_500_000_000,
        egress_bytes: null,
      },
    });
    expect(page.stats.map((stat) => [stat.label, stat.value])).toEqual([
      ['CPU', '1,000 vCPU-min'],
      ['Disk', '2.50 GB'],
    ]);
  });

  it('keeps small compute readable instead of 0.0', () => {
    expect(computeTime(0, true)).toBe('Under 1 vCPU-sec');
    expect(computeTime(3, true)).toBe('3 vCPU-sec');
    expect(computeTime(600, true)).toBe('10 vCPU-min');
    expect(computeTime(90, false)).toBe('90 sec');
  });
});
