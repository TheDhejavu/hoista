/**
 * Shapes shared by every part of the local API.
 *
 * They follow the engine's API: money as integer cents with a currency, times
 * as RFC 3339 UTC strings, lists in pages with a `cursor`, and errors with a
 * `message` that's safe to show. Field names are snake_case because they are
 * the engine's JSON, unchanged.
 */
import type { Provider } from '@engine-api';
import type { SoonId } from '../providers/soon';

/** An RFC 3339 UTC timestamp, for example `2026-11-02T04:10:00Z`. */
export type Timestamp = string;

/** An amount of money. Never a float. */
export interface Money {
  cents: number;
  currency: 'USD';
}

/**
 * A provider: one the engine has an adapter for (its `Provider`), or one the app
 * lists as coming soon.
 */
export type ProviderId = Provider | SoonId;

/** A page of results. `cursor` is null on the last page. */
export interface Page<T> {
  items: T[];
  cursor: string | null;
}

/** An error from the engine or from a provider, already worded for people. */
export interface ApiError {
  code: string;
  message: string;
  provider_detail?: string;
}

/** HTTP methods the local API uses. */
export type Method = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';

/** One request to the local API, as the screens send it. */
export interface EngineRequest {
  method: Method;
  /** Path with query string, for example `/projects?status=live`. */
  path: string;
  body?: unknown;
}

/** The result of one request. The bridge never throws across IPC. */
export type EngineResult<T = unknown> = { ok: true; data: T } | { ok: false; error: ApiError };
