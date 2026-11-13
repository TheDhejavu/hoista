import { describe, expect, it } from 'vitest';
import type { Settings as DaemonSettings } from '@engine-api';
import type { EngineRequest, Settings } from '@shared/api';
import { unwrap, type Engine } from '@shared/engine';
import { createAdapter } from '.';
import { memoryPrefs } from './prefs';
import { toEnginePatch, toSettings } from './settings';

const engineSettings: DaemonSettings = {
  agents_enabled: true,
  idle_days: 21,
  agent_expiry_days: 30,
  check_updates: true,
  crash_reports: false,
  network_page: false,
  mcp_http: true,
  default_access: 'ask',
  unisolated_builds: false,
  backup_every_hours: 24,
  backup_keep: 14,
  history_days: 365,
  server: null,
};

describe('settings from the daemon', () => {
  it('reads engine settings and desktop preferences into one view', () => {
    const view = toSettings(
      engineSettings,
      { theme: 'dark', open_at_login: 'window', notify: 'never' },
      '0.1.0',
    );
    expect(view.general.theme).toBe('dark');
    expect(view.agents.disconnect_unused_after_days).toBe(30);
    expect(view.safety.on_safety_problem).toBe('stop');
    expect(view.agents.remote.status).toBe('off');
    expect(view.updates.current_version).toBe('0.1.0');
  });

  it('refuses what the engine cannot store instead of pretending to save', () => {
    expect(() => toEnginePatch({ agents: { disconnect_unused_after_days: null } })).toThrow(
      /at most a year/,
    );
    expect(() => toEnginePatch({ updates: { channel: 'beta' } })).toThrow(/beta/);
  });

  it('keeps theme on this computer and never half-applies a refused patch', async () => {
    const asked: EngineRequest[] = [];
    const daemon: Engine = {
      request: (request) => {
        asked.push(request);
        if (request.path === '/daemon') {
          return Promise.resolve({ ok: true, data: { version: '0.1.0', mcp_command: [] } });
        }
        return Promise.resolve({ ok: true, data: engineSettings });
      },
      subscribe: () => () => undefined,
    };
    const prefs = memoryPrefs();
    const adapter = createAdapter(daemon, { prefs });
    const saved = unwrap<Settings>(
      await adapter.request({
        method: 'PATCH',
        path: '/settings',
        body: { general: { theme: 'light' } },
      }),
    );
    expect(saved.general.theme).toBe('light');
    expect(asked.some((request) => request.method === 'PATCH')).toBe(false);

    const refused = await adapter.request({
      method: 'PATCH',
      path: '/settings',
      body: { general: { theme: 'dark' }, updates: { channel: 'beta' } },
    });
    expect(refused.ok).toBe(false);
    expect(prefs.read().theme).toBe('light');
  });

  it('turns on building without isolation only when asked', () => {
    expect(toEnginePatch({ safety: { build_without_isolation: true } })).toEqual({
      unisolated_builds: true,
    });
    expect(toSettings(engineSettings, memoryPrefs().read(), '1.0.0').safety).toMatchObject({
      build_without_isolation: false,
    });
  });
});
