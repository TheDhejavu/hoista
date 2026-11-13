/**
 * A small choice remembered on this computer, such as the Projects view or a
 * filter. Storage can be missing or full, so reads fall back and writes are
 * best-effort. When `key` changes (another project's filters, say), the
 * choice saved under the new key is read.
 */
import { useState } from 'react';

export function useStoredChoice<T>(
  key: string,
  fallback: T,
  isValid: (value: unknown) => value is T,
): [T, (value: T) => void] {
  const read = (): T => {
    try {
      const raw = window.localStorage.getItem(key);
      if (raw === null) return fallback;
      const parsed: unknown = JSON.parse(raw);
      return isValid(parsed) ? parsed : fallback;
    } catch {
      return fallback;
    }
  };
  const [state, setState] = useState<{ key: string; value: T }>(() => ({ key, value: read() }));
  let value = state.value;
  if (state.key !== key) {
    // Another key since the last render: show its choice from this render on.
    value = read();
    setState({ key, value });
  }

  function update(next: T): void {
    setState({ key, value: next });
    try {
      window.localStorage.setItem(key, JSON.stringify(next));
    } catch {
      // Not remembered this time; the choice still applies until Hoista closes.
    }
  }

  return [value, update];
}

/** A filter remembered on this computer: one of `choices`, or nothing picked. */
export function useStoredFilter<T extends string | number>(
  key: string,
  fallback: T | null,
  choices?: readonly T[],
): [T | null, (value: T | null) => void] {
  return useStoredChoice<T | null>(
    `hoista.filter.${key}`,
    fallback,
    (value): value is T | null =>
      value === null ||
      (choices
        ? choices.includes(value as T)
        : typeof value === 'string' || typeof value === 'number'),
  );
}

/** A choice from a list of options (a sort, a time range), remembered on this computer. */
export function useStoredOption<T>(
  key: string,
  fallback: T,
  options: readonly { value: T }[],
): [T, (value: T) => void] {
  return useStoredChoice<T>(`hoista.filter.${key}`, fallback, (value): value is T =>
    options.some((option) => option.value === value),
  );
}
