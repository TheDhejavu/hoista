import { describe, expect, it } from 'vitest';
import type { EngineRequest, Settings } from '@shared/api';
import { unwrap, type Engine } from '@shared/engine';
import { noUpdater, withUpdates, type Updater } from './updates';

/** Settings as the adapter reads them, before the updater fills in its part. */
const fixture: Settings = {
  general: { theme: 'system', open_at_login: 'menu_bar', notify: 'deploys' },
  agents: {
    connection_enabled: true,
    network_page: false,
    mcp_http: true,
    default_access: 'ask',
    disconnect_unused_after_days: 30,
    approval_timeout_minutes: 15,
    remote: {
      target: 'this_computer',
      status: 'off',
      last_sync: null,
      token_access: 'full',
      url: null,
    },
  },
  safety: {
    on_safety_problem: 'stop',
    builds_isolated_by: 'the macOS sandbox',
    builds_not_isolated_because: null,
    build_without_isolation: false,
  },
  backups: { every_hours: 24, keep: 14, history_days: 365 },
  updates: {
    channel: 'stable',
    auto_check: true,
    current_version: '0.1.0',
    latest_version: null,
    latest_summary: null,
    latest_notes: [],
    checked_at: null,
    can_update: true,
  },
  cli: {
    installed: true,
    path: '/usr/local/bin/hoist',
    version: '0.1.0',
    mcp_command: 'hoist mcp',
    mcp_args: ['hoist', 'mcp'],
  },
};

function engine(asked: EngineRequest[]): Engine {
  return {
    request: (request) => {
      asked.push(request);
      return Promise.resolve({ ok: true, data: structuredClone(fixture) });
    },
    subscribe: () => () => undefined,
  };
}

function releaseUpdater(): Updater & { installed: boolean } {
  const state = {
    channel: 'stable' as 'stable' | 'beta',
    latest_version: null as string | null,
    latest_summary: null as string | null,
    latest_notes: [] as string[],
    checked_at: null as string | null,
  };
  return {
    installed: false,
    available: () => true,
    state: () => state,
    check() {
      state.latest_version = '0.5.0';
      state.latest_notes = ['Webhooks screen'];
      state.checked_at = '2026-11-04T19:00:00Z';
      return Promise.resolve();
    },
    install() {
      this.installed = true;
      return Promise.resolve();
    },
    setChannel(channel) {
      state.channel = channel;
      return Promise.resolve();
    },
  };
}

describe('updates', () => {
  it('says plainly when this copy can’t update itself', async () => {
    const project = withUpdates(engine([]), noUpdater);
    const shown = unwrap<Settings>(await project.request({ method: 'GET', path: '/settings' }));
    expect(shown.updates.can_update).toBe(false);
    expect(shown.updates.latest_version).toBeNull();
    const check = await project.request({ method: 'POST', path: '/updates/check' });
    expect(check.ok ? null : check.error.code).toBe('updates.unavailable');
  });

  it('shows what the updater found, and installs only something newer', async () => {
    const updater = releaseUpdater();
    const project = withUpdates(engine([]), updater);
    const early = await project.request({ method: 'POST', path: '/updates/install' });
    expect(early.ok ? null : early.error.code).toBe('updates.none');
    const checked = unwrap<Settings>(
      await project.request({ method: 'POST', path: '/updates/check' }),
    );
    expect(checked.updates.latest_version).toBe('0.5.0');
    expect(checked.updates.can_update).toBe(true);
    await project.request({ method: 'POST', path: '/updates/install' });
    expect(updater.installed).toBe(true);
  });

  it('keeps the channel with the updater, never the engine', async () => {
    const asked: EngineRequest[] = [];
    const updater = releaseUpdater();
    const project = withUpdates(engine(asked), updater);
    await project.request({
      method: 'PATCH',
      path: '/settings',
      body: { updates: { channel: 'beta', auto_check: true } },
    });
    expect(updater.state().channel).toBe('beta');
    expect(asked[0]?.body).toEqual({ updates: { auto_check: true } });
  });
});
