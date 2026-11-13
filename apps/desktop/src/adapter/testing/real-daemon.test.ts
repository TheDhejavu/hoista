/**
 * Runs the adapter against a real `hoistd` over its socket. Opt-in: set
 * HOISTD to the daemon binary (for example `target/debug/hoistd`). Uses a
 * temporary state directory, so it never touches your own Hoista state.
 */
import { spawn, type ChildProcess } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import type { Project } from '@engine-api';
import type { ProjectDetail, ProjectList, EngineEvent, Profile, Settings } from '@shared/api';
import { unwrap, type Engine } from '@shared/engine';
import { createSocketEngine } from './socket-engine';
import { createAdapter } from '..';

const binary = process.env.HOISTD;

describe.skipIf(!binary)('adapter against a real hoistd', () => {
  let state = '';
  let daemon: ChildProcess | undefined;
  let engine: Engine;

  beforeAll(async () => {
    state = mkdtempSync(join(tmpdir(), 'hoista-desktop-'));
    daemon = spawn(binary ?? '', ['--state-dir', state], { stdio: 'ignore' });
    const socket = join(state, 'hoistd.sock');
    for (let tries = 0; !existsSync(socket); tries += 1) {
      if (tries > 100) throw new Error('hoistd did not open its socket');
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    engine = createAdapter(createSocketEngine(socket));
  });

  afterAll(() => {
    daemon?.kill('SIGTERM');
    rmSync(state, { recursive: true, force: true });
  });

  it('starts at first run, saves a profile and streams only new events', async () => {
    expect(unwrap(await engine.request({ method: 'GET', path: '/profile' }))).toBeNull();

    const events: EngineEvent[] = [];
    const stop = engine.subscribe((event) => events.push(event));
    await new Promise((resolve) => setTimeout(resolve, 300));
    const saved = unwrap<Profile>(
      await engine.request({
        method: 'PUT',
        path: '/profile',
        body: { name: 'Ada Lovelace' },
      }),
    );
    expect(saved).toEqual({ name: 'Ada Lovelace', initials: 'AL' });
    for (let tries = 0; events.length === 0 && tries < 40; tries += 1) {
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    stop();
    expect(events.map((event) => event.kind)).toEqual(['profile.updated']);
  });

  it('lists no projects on a fresh engine, with unknown cost', async () => {
    const list = unwrap<ProjectList>(await engine.request({ method: 'GET', path: '/projects' }));
    expect(list.items).toEqual([]);
    expect(list.totals).toEqual({ projects: 0, live: 0, failing: 0, providers: 0 });
  });

  it('shows a new project on the list and its overview', async () => {
    const created = unwrap<Project>(
      await engine.request({
        method: 'POST',
        path: '/projects',
        body: { name: 'notes', source: { kind: 'unlinked' }, mode: 'managed' },
      }),
    );
    const list = unwrap<ProjectList>(await engine.request({ method: 'GET', path: '/projects' }));
    expect(list.items.map((item) => item.name)).toEqual(['notes']);
    expect(list.totals.projects).toBe(1);
    const detail = unwrap<ProjectDetail>(
      await engine.request({ method: 'GET', path: `/projects/${created.id}` }),
    );
    expect(detail.project).toMatchObject({ name: 'notes', source: { kind: 'unlinked', ref: '' } });
    expect(detail.resources).toEqual([]);
    const recent = unwrap<{ items: unknown[] }>(
      await engine.request({ method: 'GET', path: '/deploys' }),
    );
    expect(recent.items).toEqual([]);
  });

  it('lists no accounts yet and refuses services the engine cannot connect', async () => {
    const accounts = unwrap<{ items: unknown[] }>(
      await engine.request({ method: 'GET', path: '/accounts' }),
    );
    expect(accounts.items).toEqual([]);
    const netlify = await engine.request({
      method: 'POST',
      path: '/accounts',
      body: { service: 'netlify', token: 'unused' },
    });
    expect(netlify.ok ? null : netlify.error.code).toBe('account.unsupported');
  });

  it('reads and saves settings through the engine', async () => {
    const before = unwrap<Settings>(await engine.request({ method: 'GET', path: '/settings' }));
    expect(before.agents.connection_enabled).toBe(true);
    const after = unwrap<Settings>(
      await engine.request({
        method: 'PATCH',
        path: '/settings',
        body: {
          agents: { connection_enabled: false, disconnect_unused_after_days: 90 },
        },
      }),
    );
    expect(after.agents).toMatchObject({
      connection_enabled: false,
      disconnect_unused_after_days: 90,
    });
    expect(after.updates.current_version).toMatch(/^\d+\.\d+\.\d+/);
  });

  it('has nothing needing attention on a fresh engine', async () => {
    const attention = unwrap<{ items: unknown[] }>(
      await engine.request({ method: 'GET', path: '/attention' }),
    );
    expect(attention.items).toEqual([]);
  });

  it('reads what the providers found, through the engine', async () => {
    const found = unwrap<{ groups: unknown[]; resources: unknown[]; combine: unknown[] }>(
      await engine.request({ method: 'GET', path: '/import/found' }),
    );
    expect(found).toEqual({ groups: [], resources: [], combine: [] });
  });
});
