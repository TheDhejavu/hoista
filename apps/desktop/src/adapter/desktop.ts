/**
 * What the adapter needs from the app around it. The app passes the real
 * ones (see `src/native`); tests and the browser preview leave them out and
 * get the defaults.
 */
import type { Engine } from '@shared/engine';
import type { OpenPage } from './accounts';
import { memoryPrefs } from './prefs';
import type { PrefsStore } from './settings';

/** The `hoist` command on this computer. */
export interface Cli {
  path: string;
  version: string | null;
}

export interface Desktop {
  /** This computer's engine, when the screens may show a remote's instead. */
  local: Engine;
  prefs: PrefsStore;
  /** Opens a provider's sign-in page in the browser. */
  openPage: OpenPage;
  /** This computer's account name, for the suggested profile name. */
  userName: string;
  findCli: () => Promise<Cli | null>;
  /** Whether a Hoista answers its health check at `url`. */
  answersAt: (url: string) => Promise<boolean>;
}

export function withDefaults(daemon: Engine, given: Partial<Desktop>): Desktop {
  return {
    local: daemon,
    prefs: memoryPrefs(),
    openPage: () => Promise.reject(new Error('No browser to open the provider’s page.')),
    userName: '',
    findCli: () => Promise.resolve(null),
    answersAt: () => Promise.resolve(false),
    ...given,
  };
}
