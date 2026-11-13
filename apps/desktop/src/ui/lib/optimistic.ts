/**
 * Changes that show on screen before the engine answers, as native apps do
 * for toggles and renames: the cached data is edited at once, and if the
 * engine refuses, it goes back and a note says why.
 *
 * Only for changes that are safe to show early and easy to undo. Anything
 * that does real work at a provider (deploys, deleting) waits for the answer.
 */
import type { QueryClient, QueryKey } from '@tanstack/react-query';
import { showToast } from './toast';

/** One cached query (or every query under a key) and how a change edits it. */
interface CacheEdit<Vars> {
  key: QueryKey;
  /** Only the query with exactly this key, not the ones under it. */
  exact?: boolean;
  apply: (old: unknown, vars: Vars) => unknown;
}

/** Builds a `CacheEdit` with the cached data's type spelled out. */
export function edit<Data, Vars>(
  key: QueryKey,
  apply: (old: Data, vars: Vars) => Data,
  options: { exact?: boolean } = {},
): CacheEdit<Vars> {
  return {
    key,
    ...(options.exact ? { exact: true } : {}),
    apply: (old, vars) => apply(old as Data, vars),
  };
}

type Snapshot = [QueryKey, unknown][];

/**
 * `onMutate`, `onError` and `onSettled` for a mutation whose result shows
 * at once. `what` names the change in the note if it's refused ("Name").
 */
export function optimistic<Vars>(client: QueryClient, what: string, edits: CacheEdit<Vars>[]) {
  return {
    onMutate: async (vars: Vars): Promise<Snapshot> => {
      const snapshot: Snapshot = [];
      for (const change of edits) {
        const filter = { queryKey: change.key, exact: change.exact ?? false };
        // A refresh already on its way would overwrite the change with old data.
        await client.cancelQueries(filter);
        snapshot.push(...client.getQueriesData(filter));
        client.setQueriesData(filter, (old: unknown) =>
          old === undefined ? old : change.apply(old, vars),
        );
      }
      return snapshot;
    },
    onError: (error: Error, _vars: Vars, snapshot: Snapshot | undefined) => {
      for (const [key, data] of snapshot ?? []) client.setQueryData(key, data);
      showToast(`${what} wasn’t saved, so it’s back as it was. ${error.message}`, {
        tone: 'error',
      });
    },
    onSettled: () =>
      Promise.all(
        edits.map((change) =>
          client.invalidateQueries({ queryKey: change.key, exact: change.exact ?? false }),
        ),
      ),
  };
}

/** Merges a partial update into an object, one level of sections deep. */
export function mergeSections<T extends object>(old: T, patch: { [K in keyof T]?: unknown }): T {
  const next: Record<string, unknown> = { ...(old as Record<string, unknown>) };
  for (const [key, value] of Object.entries(patch)) {
    const current = next[key];
    next[key] = isPlainObject(current) && isPlainObject(value) ? { ...current, ...value } : value;
  }
  return next as T;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
