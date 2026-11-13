/**
 * Settings · Updates: checking for and installing new versions of Hoista
 * Desktop. The app's updater does the work; this answers the routes the screen uses and fills the update fields of
 * `GET /settings` with what the updater really found. A copy that wasn't
 * built to update itself says so instead of pretending to check.
 */
import type { EngineResult, Settings } from '@shared/api';
import type { Engine } from '@shared/engine';
import { RouteError, Router } from '@shared/router';

/** What the updater knows. */
export interface UpdateState {
  channel: 'stable' | 'beta';
  latest_version: string | null;
  latest_summary: string | null;
  latest_notes: string[];
  checked_at: string | null;
}

/** The desktop's updater, as these routes need it. */
export interface Updater {
  /** Whether this copy can update itself: a signed release with an update feed. */
  available(): boolean;
  state(): UpdateState;
  /** Asks the update feed for a newer version. */
  check(): Promise<void>;
  /** Downloads the newer version, checks its signature, then restarts into it. */
  install(): Promise<void>;
  setChannel(channel: 'stable' | 'beta'): Promise<void>;
}

/** An updater for a copy that can't update itself, such as a development build. */
export const noUpdater: Updater = {
  available: () => false,
  state: () => ({
    channel: 'stable',
    latest_version: null,
    latest_summary: null,
    latest_notes: [],
    checked_at: null,
  }),
  check: () => Promise.resolve(),
  install: () => Promise.resolve(),
  setChannel: () => Promise.resolve(),
};

const UNAVAILABLE =
  'This copy of Hoista can’t update itself. Download new versions from hoista.dev.';

/** The update fields of the settings, from the updater. */
function withState(settings: Settings, updater: Updater): Settings {
  const state = updater.state();
  return {
    ...settings,
    updates: {
      ...settings.updates,
      ...state,
      can_update: updater.available(),
    },
  };
}

export function withUpdates(engine: Engine, updater: Updater): Engine {
  const router = new Router();
  const settings = async (): Promise<Settings> => {
    const result = await engine.request({ method: 'GET', path: '/settings' });
    if (!result.ok) throw new RouteError(result.error.code, result.error.message);
    return withState(result.data as Settings, updater);
  };
  router.on('POST', '/updates/check', async (): Promise<Settings> => {
    if (!updater.available()) throw new RouteError('updates.unavailable', UNAVAILABLE);
    await updater.check();
    return settings();
  });
  router.on('POST', '/updates/install', async (): Promise<Settings> => {
    if (!updater.available()) throw new RouteError('updates.unavailable', UNAVAILABLE);
    if (!updater.state().latest_version) {
      throw new RouteError('updates.none', 'There’s no newer version to install.');
    }
    // Hoista restarts once it's downloaded and checked; until then the dialog shows it working.
    await updater.install();
    return settings();
  });

  return {
    async request(request): Promise<EngineResult> {
      if (router.handles(request)) return router.dispatch(request, () => undefined);
      const patch = request.body as { updates?: { channel?: 'stable' | 'beta' } } | undefined;
      // The channel is the updater's, never the engine's.
      if (request.method === 'PATCH' && request.path === '/settings' && patch?.updates?.channel) {
        if (!updater.available()) {
          return { ok: false, error: { code: 'updates.unavailable', message: UNAVAILABLE } };
        }
        await updater.setChannel(patch.updates.channel);
        const rest = { ...patch.updates };
        delete rest.channel;
        const body = { ...patch, updates: rest };
        const result = await engine.request({ ...request, body });
        return result.ok ? { ok: true, data: withState(result.data as Settings, updater) } : result;
      }
      const result = await engine.request(request);
      if (result.ok && request.path === '/settings' && result.data) {
        return { ok: true, data: withState(result.data as Settings, updater) };
      }
      return result;
    },
    subscribe: (listener) => engine.subscribe(listener),
    ...(engine.stream ? { stream: engine.stream.bind(engine) } : {}),
  };
}
