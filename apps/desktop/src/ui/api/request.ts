/**
 * Sends one request to the local API and unwraps the result.
 *
 * Returns the data on success and throws an `EngineError` on failure, so
 * React Query can show error states. The error's `message` is already plain
 * English and safe to show.
 */
import type { Method } from '@shared/api';
import { unwrap } from '@shared/engine';
import { getBridge } from './bridge';

export async function request<T>(method: Method, path: string, body?: unknown): Promise<T> {
  return unwrap<T>(await getBridge().request({ method, path, body }));
}

/** `apiPath('projects', id, 'secrets', name)` → `/projects/<id>/secrets/<name>`, each resource encoded. */
export function apiPath(...segments: string[]): string {
  return `/${segments.map(encodeURIComponent).join('/')}`;
}

/** Builds `?a=1&b=2` from the defined values only. */
export function queryString(params: Record<string, string | undefined>): string {
  const defined = Object.entries(params).filter(
    (entry): entry is [string, string] => entry[1] !== undefined && entry[1] !== '',
  );
  return defined.length ? `?${new URLSearchParams(defined).toString()}` : '';
}
