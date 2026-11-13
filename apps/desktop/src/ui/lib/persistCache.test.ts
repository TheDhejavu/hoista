import { describe, expect, it } from 'vitest';
import { isKept } from './persistCache';
import { mergeSections } from './optimistic';

describe('isKept', () => {
  it('keeps lists and screen data', () => {
    expect(isKept(['projects', 'list', { sort: 'last_deploy' }])).toBe(true);
    expect(isKept(['projects', 'moodboard'])).toBe(true);
    expect(isKept(['projects', 'moodboard', 'map'])).toBe(true);
    expect(isKept(['projects', 'moodboard', 'domains'])).toBe(true);
    expect(isKept(['accounts'])).toBe(true);
    expect(isKept(['agents'])).toBe(true);
    expect(isKept(['deploys'])).toBe(true);
  });

  it('never keeps variables, logs or anything private', () => {
    expect(isKept(['projects', 'moodboard', 'secrets'])).toBe(false);
    expect(isKept(['projects', 'moodboard', 'logs', { q: '' }])).toBe(false);
    expect(isKept(['deploys', 'd1', 'log'])).toBe(false);
    expect(isKept(['accounts', 'acc_1'])).toBe(false);
    expect(isKept(['settings'])).toBe(false);
    expect(isKept(['settings', 'vault'])).toBe(false);
    expect(isKept(['agents', 'activity', 'all'])).toBe(false);
    expect(isKept(['projects', 'found'])).toBe(false);
    expect(isKept(['provider', 'r1'])).toBe(false);
  });
});

describe('mergeSections', () => {
  it('merges one level of sections and replaces the rest', () => {
    const old = { agents: { connection_enabled: true, idle_days: 7 }, theme: 'system' };
    expect(mergeSections(old, { agents: { connection_enabled: false } })).toEqual({
      agents: { connection_enabled: false, idle_days: 7 },
      theme: 'system',
    });
    expect(mergeSections(old, { theme: 'dark' })).toEqual({ ...old, theme: 'dark' });
  });
});
