/** The app's own preferences: read once at startup, and saved by Rust on each change. */
import { invoke } from '@tauri-apps/api/core';
import type { DesktopPrefs, PrefsStore } from '@adapter/settings';
import { cleanPrefs, memoryPrefs } from '@adapter/prefs';

export function savedPrefs(saved: Partial<DesktopPrefs> | null): PrefsStore {
  const prefs = memoryPrefs(cleanPrefs(saved ?? {}));
  return {
    read: () => prefs.read(),
    write: (next) => {
      prefs.write(next);
      void invoke('preferences_save', { preferences: next });
    },
  };
}
