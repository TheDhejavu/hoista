/**
 * Keeps what the app last saw on this computer, so it opens on your
 * projects, accounts and agents straight away and refreshes them quietly,
 * the way Mail or Linear do. Only a first launch shows loading.
 *
 * What's kept is the screen data only: names, statuses, lists. Variable
 * values, logs, tokens and anything else private are never written. It's
 * saved in the app's own storage on this computer, and cleared when you
 * switch to another instance so one never shows the other's data.
 */
import {
  dehydrate,
  hydrate,
  type DehydratedState,
  type Query,
  type QueryClient,
} from '@tanstack/react-query';

const KEY = 'hoista.cache';
/** When the save was last dropped. A window opened before then mustn't write its old data back. */
const CLEARED = 'hoista.cache.cleared';
/** Bumped when the shape of saved data changes, so old saves are dropped. */
const VERSION = 1;
/** Older than this, a save is more confusing than useful. */
const MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000;
const SAVE_DELAY_MS = 1_000;

interface Saved {
  version: number;
  savedAt: number;
  state: DehydratedState;
}

/** Project screens whose data is safe to keep. Never variables or logs. */
const PROJECT_PARTS = new Set(['map', 'deploys', 'domains', 'settings', 'activity']);

/** Whether a query's data is kept between launches. */
export function isKept(key: readonly unknown[]): boolean {
  const [root, second, third] = key;
  switch (root) {
    case 'projects':
      if (second === 'list') return true;
      if (typeof second !== 'string' || second === 'found' || second === 'resources') return false;
      return key.length === 2 || (key.length === 3 && PROJECT_PARTS.has(third as string));
    case 'deploys':
    case 'environments':
    case 'attention':
    case 'catalogue':
    case 'profile':
      return key.length === 1;
    case 'accounts':
      return key.length === 1 || (key.length === 2 && second === 'bills');
    case 'agents':
      return key.length === 1;
    default:
      return false;
  }
}

/** At start, before the first screen: puts the last saved data back. */
export function restoreCache(client: QueryClient): void {
  try {
    const raw = window.localStorage.getItem(KEY);
    if (!raw) return;
    const saved = JSON.parse(raw) as Saved;
    if (saved.version !== VERSION || Date.now() - saved.savedAt > MAX_AGE_MS) {
      forgetCache();
      return;
    }
    hydrate(client, saved.state);
  } catch {
    forgetCache();
  }
}

/**
 * Saves the kept data a moment after it changes, and when the window goes.
 * Every Hoista window adds what it has to the same save, so the menu bar
 * panel never wipes what the main window knows. Returns a function that
 * stops saving.
 */
export function keepCache(client: QueryClient): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const save = (): void => {
    clearTimeout(timer);
    timer = undefined;
    try {
      const cleared = Number(window.localStorage.getItem(CLEARED) ?? 0);
      if (cleared > performance.timeOrigin) return;
      const now = dehydrate(client, {
        shouldDehydrateQuery: (query) => query.state.status === 'success' && isKept(query.queryKey),
        shouldDehydrateMutation: () => false,
      });
      const before = readState();
      const fresh = new Set(now.queries.map((query) => query.queryHash));
      const queries = [
        ...(before?.queries ?? []).filter((query) => !fresh.has(query.queryHash)),
        ...now.queries,
      ].filter((query) => isKept(query.queryKey));
      const saved: Saved = {
        version: VERSION,
        savedAt: Date.now(),
        state: { mutations: [], queries },
      };
      window.localStorage.setItem(KEY, JSON.stringify(saved));
    } catch {
      // Storage full or missing: the app still works, it just starts fresh next time.
    }
  };
  const later = (): void => {
    clearTimeout(timer);
    timer = setTimeout(save, SAVE_DELAY_MS);
  };
  const stop = client.getQueryCache().subscribe((event) => {
    if (event.type !== 'updated' || event.action.type !== 'success') return;
    const key = (event.query as Query).queryKey;
    if (isKept(key)) later();
  });
  const onHide = (): void => {
    if (timer !== undefined) save();
  };
  window.addEventListener('pagehide', onHide);
  return () => {
    stop();
    clearTimeout(timer);
    window.removeEventListener('pagehide', onHide);
  };
}

/** Drops the save, such as when switching to another instance. */
export function forgetCache(): void {
  try {
    window.localStorage.removeItem(KEY);
    window.localStorage.setItem(CLEARED, String(Date.now()));
  } catch {
    // Nothing saved, or no storage: either way nothing will be restored.
  }
}

function readState(): DehydratedState | null {
  try {
    const raw = window.localStorage.getItem(KEY);
    if (!raw) return null;
    const saved = JSON.parse(raw) as Saved;
    return saved.version === VERSION ? saved.state : null;
  } catch {
    return null;
  }
}
