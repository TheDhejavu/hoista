/**
 * The app's own preferences (theme, open at login, notifications). The app
 * saves them to a file; here they're kept in memory and checked.
 */
import type { Settings } from '@shared/api';
import { DEFAULT_PREFS, type DesktopPrefs, type PrefsStore } from './settings';

const THEMES: readonly Settings['general']['theme'][] = ['dark', 'light', 'system'];
const LOGIN: readonly Settings['general']['open_at_login'][] = ['menu_bar', 'window', 'off'];
const NOTIFY: readonly Settings['general']['notify'][] = ['deploys', 'problems', 'never'];

/** Keeps known values only, so a hand-edited file cannot break the app. */
export function cleanPrefs(value: Partial<DesktopPrefs>): DesktopPrefs {
  return {
    theme: THEMES.includes(value.theme as DesktopPrefs['theme'])
      ? (value.theme as DesktopPrefs['theme'])
      : DEFAULT_PREFS.theme,
    open_at_login: LOGIN.includes(value.open_at_login as DesktopPrefs['open_at_login'])
      ? (value.open_at_login as DesktopPrefs['open_at_login'])
      : DEFAULT_PREFS.open_at_login,
    notify: NOTIFY.includes(value.notify as DesktopPrefs['notify'])
      ? (value.notify as DesktopPrefs['notify'])
      : DEFAULT_PREFS.notify,
  };
}

/** Preferences kept in memory. */
export function memoryPrefs(initial: DesktopPrefs = DEFAULT_PREFS): PrefsStore {
  let prefs = initial;
  return {
    read: () => prefs,
    write: (next) => {
      prefs = next;
    },
  };
}
