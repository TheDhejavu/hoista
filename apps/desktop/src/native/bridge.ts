/**
 * `window.hoista`: everything the screens can ask of the app. Requests go
 * through the adapter, which shapes `hoistd`'s answers for the screens, to
 * Rust, which sends them to whichever Hoista is shown.
 */
import { invoke } from '@tauri-apps/api/core';
import { downloadDir, join } from '@tauri-apps/api/path';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { open, save } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';
import type { HoistaBridge, Platform } from '@shared/bridge';
import type { Engine } from '@shared/engine';
import { COMMAND_PREFIX } from '@shared/shortcuts';
import { createAdapter, type Cli } from '@adapter/index';
import type { DesktopPrefs } from '@adapter/settings';
import { noUpdater, withUpdates } from '@adapter/updates';
import { startMainWindow } from './desktop';
import { localEngine, shownEngine } from './engine';
import { savedPrefs } from './prefs';
import { withRemotes } from './remotes';
import { isExternalUrl, isWindowTarget } from './validate';

/** What Rust tells the page before the screens start. */
export interface Startup {
  platform: Platform;
  user: string;
  preferences: Partial<DesktopPrefs> | null;
  start_at: string | null;
  tray: boolean;
}

const MAIN = 'main';
const TRANSFER_FILES = [{ name: 'Hoista transfer', extensions: ['hoista'] }];

function openExternal(url: string): Promise<void> {
  if (!isExternalUrl(url)) return Promise.reject(new Error('Only web links open.'));
  return openUrl(url);
}

/** A provider's sign-in page opens in the browser, and only a secure one. */
function openPage(url: string): Promise<void> {
  if (!url.startsWith('https://')) {
    return Promise.reject(new Error('Hoista only opens secure provider pages.'));
  }
  return openUrl(url);
}

function connect(startup: Startup): Engine {
  const adapter = createAdapter(shownEngine(), {
    local: localEngine(),
    prefs: savedPrefs(startup.preferences),
    openPage,
    userName: startup.user,
    findCli: () => invoke<Cli | null>('find_cli'),
    answersAt: (url) => invoke<boolean>('remote_answers', { url }).catch(() => false),
  });
  return withUpdates(withRemotes(adapter), noUpdater);
}

export function createBridge(startup: Startup): HoistaBridge {
  const engine = connect(startup);
  const isMain = getCurrentWebviewWindow().label === MAIN;
  const navigateListeners = new Set<(target: string) => void>();
  const navigate = (target: string): void => {
    for (const listener of navigateListeners) listener(target);
  };

  if (isMain) {
    // Menus and the panel send screens and commands here through Rust.
    void getCurrentWebviewWindow().listen<string>('navigate', ({ payload }) => {
      if (payload.startsWith(COMMAND_PREFIX) || isWindowTarget(payload)) navigate(payload);
    });
    startMainWindow(engine, startup.tray);
  }

  return {
    platform: startup.platform,
    request: (request) => engine.request(request),
    onEvent: (listener) => engine.subscribe(listener),
    openExternal,
    chooseFolder: async () => {
      const picked = await open({ title: 'Choose a project folder', directory: true });
      return typeof picked === 'string' ? picked : null;
    },
    chooseTransferFile: async () => {
      const picked = await open({
        title: 'Choose the file from your old computer',
        filters: TRANSFER_FILES,
      });
      return typeof picked === 'string' ? picked : null;
    },
    // A webview's File has no path; the screens then ask to choose the file.
    pathForFile: () => null,
    chooseTransferSave: async () => {
      const name = `hoista-${new Date().toISOString().slice(0, 10)}.hoista`;
      const picked = await save({
        title: 'Save the file for your new computer',
        defaultPath: await join(await downloadDir(), name).catch(() => name),
        filters: TRANSFER_FILES,
      });
      if (!picked) return null;
      return picked.endsWith('.hoista') ? picked : `${picked}.hoista`;
    },
    quit: () => invoke('quit'),
    showMain: (target) => invoke('show_main', { target: target ?? null }),
    onNavigate: (listener) => {
      navigateListeners.add(listener);
      return () => navigateListeners.delete(listener);
    },
    closePanel: () => invoke('panel_hide'),
    resizePanel: (height) => invoke('panel_resize', { height }),
    reloadWindows: () => invoke('windows_reload'),
  };
}
