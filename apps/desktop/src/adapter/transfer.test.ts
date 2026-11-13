import { describe, expect, it } from 'vitest';
import type { EngineRequest, TransferFile, TransferImported } from '@shared/api';
import { unwrap, type Engine } from '@shared/engine';
import { createAdapter } from '.';
import { contentsOf } from './transfer';

const inspection = {
  created_at: '2026-11-04T18:00:00Z',
  has_profile: true,
  accounts: 2,
  credentials: 2,
  projects: 3,
  resources: 7,
  ignored_resources: 0,
  activity: 40,
};

function daemon(asked: EngineRequest[]): Engine {
  return {
    request: (request) => {
      asked.push(request);
      if (request.path === '/transfer/export') {
        return Promise.resolve({ ok: true, data: { path: '/Users/sam/Downloads/a.hoista' } });
      }
      if (request.path === '/transfer/inspect') {
        return Promise.resolve({ ok: true, data: inspection });
      }
      if (request.path === '/profile') {
        return Promise.resolve({ ok: true, data: { name: 'Sam Rivera' } });
      }
      return Promise.resolve({ ok: true, data: { restored: true } });
    },
    subscribe: () => () => undefined,
  };
}

describe('moving to a new computer', () => {
  it('lists only what the file really holds', () => {
    expect(contentsOf(inspection)).toEqual([
      'Your profile and settings',
      '3 projects and 7 resources, with their maps',
      '2 provider accounts and 2 sign-ins',
      '40 activity entries',
    ]);
  });

  it('saves where the person chose, then reads the file back', async () => {
    const asked: EngineRequest[] = [];
    const adapter = createAdapter(daemon(asked));
    const file = unwrap<TransferFile>(
      await adapter.request({
        method: 'POST',
        path: '/transfer/export',
        body: { path: '/Users/sam/Downloads/a.hoista', passphrase: 'correct horse battery' },
      }),
    );
    expect(file.path).toBe('/Users/sam/Downloads/a.hoista');
    expect(file.contents).toContain('2 provider accounts and 2 sign-ins');
    expect(asked.map((request) => request.path)).toEqual(['/transfer/export', '/transfer/inspect']);
  });

  it('checks the file before restoring, and refuses a short passphrase', async () => {
    const asked: EngineRequest[] = [];
    const adapter = createAdapter(daemon(asked));
    const short = await adapter.request({
      method: 'POST',
      path: '/transfer/import',
      body: { path: '/tmp/a.hoista', passphrase: 'short' },
    });
    expect(short.ok).toBe(false);
    expect(asked).toEqual([]);
    const imported = unwrap<TransferImported>(
      await adapter.request({
        method: 'POST',
        path: '/transfer/import',
        body: { path: '/tmp/a.hoista', passphrase: 'correct horse battery' },
      }),
    );
    expect(imported).toEqual({
      profile_name: 'Sam Rivera',
      projects: 3,
      accounts: 2,
    });
    expect(asked.map((request) => request.path)).toEqual([
      '/transfer/inspect',
      '/transfer/restore',
      '/profile',
    ]);
  });
});
