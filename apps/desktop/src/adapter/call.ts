/**
 * Calls `hoistd` from adapter routes. A daemon error is rethrown as a
 * {@link RouteError} with the daemon's own code and message, so the UI shows
 * exactly what the engine said.
 */
import type { Method } from '@shared/api';
import type { Engine } from '@shared/engine';
import { RouteError } from '@shared/router';

export type Call = <T>(method: Method, path: string, body?: unknown) => Promise<T>;

export function caller(daemon: Engine): Call {
  return async <T>(method: Method, path: string, body?: unknown): Promise<T> => {
    const result = await daemon.request({ method, path, body });
    if (!result.ok) throw new RouteError(result.error.code, result.error.message);
    return result.data as T;
  };
}

/** Like {@link Call}, but a 404-style `*.not_found` answer becomes null. */
export async function orNull<T>(request: Promise<T>): Promise<T | null> {
  try {
    return await request;
  } catch (error) {
    if (error instanceof RouteError && error.error.code.endsWith('not_found')) return null;
    throw error;
  }
}

/** A query string from the defined values, for daemon paths. */
export function query(values: Record<string, string | null | undefined>): string {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(values)) {
    if (value !== null && value !== undefined && value !== '') params.set(key, value);
  }
  const text = params.toString();
  return text ? `?${text}` : '';
}

/** Pages followed before giving up: more than anyone keeps in Hoista. */
const MAX_PAGES = 40;

/**
 * Every item of a paged daemon list, following `next_cursor`. Bounded, so a
 * cursor that never ends can't hang a screen.
 */
export async function allPages<T>(call: Call, path: string): Promise<T[]> {
  const items: T[] = [];
  let cursor: string | null = null;
  for (let page = 0; page < MAX_PAGES; page += 1) {
    const separator = path.includes('?') ? '&' : '?';
    const answer: { items: T[]; next_cursor: string | null } = await call(
      'GET',
      cursor ? `${path}${separator}cursor=${encodeURIComponent(cursor)}` : path,
    );
    items.push(...answer.items);
    if (!answer.next_cursor || answer.next_cursor === cursor) return items;
    cursor = answer.next_cursor;
  }
  return items;
}
