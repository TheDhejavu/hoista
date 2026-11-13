/**
 * A small method-and-path router, used by the adapter that translates the
 * desktop's routes onto `hoistd`.
 *
 * Patterns use `:name` segments, for example `/projects/:id/deploys`. Handlers get
 * the path params, the parsed query and the request body, and return data or
 * throw a {@link RouteError} to produce an API error.
 */
import type { ApiError, EngineEvent, EngineRequest, EngineResult, Method } from './api';

/** What a handler receives. */
export interface RouteContext {
  params: Record<string, string>;
  query: URLSearchParams;
  body: unknown;
  /** Send a live event to every subscriber, as `GET /events` would. */
  emit: (event: EngineEvent) => void;
}

/** A route handler. May be async to simulate work. */
type Handler = (ctx: RouteContext) => unknown;

/** Thrown by handlers to return `{ ok: false, error }`. */
export class RouteError extends Error {
  readonly error: ApiError;

  constructor(code: string, message: string) {
    super(message);
    this.error = { code, message };
  }
}

interface Route {
  method: Method;
  resources: string[];
  handler: Handler;
}

/** Registers routes and dispatches requests to them. */
export class Router {
  readonly #routes: Route[] = [];

  /** Adds a route. First matching route wins. */
  on(method: Method, pattern: string, handler: Handler): this {
    this.#routes.push({ method, resources: split(pattern), handler });
    return this;
  }

  /** True when a route matches the method and path. */
  handles(request: EngineRequest): boolean {
    const resources = split(new URL(request.path, 'http://hoistd.local').pathname);
    return this.#routes.some(
      (route) => route.method === request.method && match(route.resources, resources) !== null,
    );
  }

  /** Runs the matching handler; unknown paths return a `not_found` error. */
  async dispatch(request: EngineRequest, emit: RouteContext['emit']): Promise<EngineResult> {
    const url = new URL(request.path, 'http://hoistd.local');
    const resources = split(url.pathname);
    for (const route of this.#routes) {
      if (route.method !== request.method) continue;
      const params = match(route.resources, resources);
      if (!params) continue;
      try {
        const data = await route.handler({
          params,
          query: url.searchParams,
          body: request.body,
          emit,
        });
        // A copy, as the real engine sends fresh JSON: handing out live state
        // would let the UI's cache share objects a handler later changes.
        return { ok: true, data: data === undefined ? null : structuredClone(data) };
      } catch (err) {
        if (err instanceof RouteError) return { ok: false, error: err.error };
        throw err;
      }
    }
    return {
      ok: false,
      error: { code: 'not_found', message: `Nothing at ${request.method} ${url.pathname}.` },
    };
  }
}

function split(path: string): string[] {
  return path.split('/').filter(Boolean);
}

function match(pattern: string[], resources: string[]): Record<string, string> | null {
  if (pattern.length !== resources.length) return null;
  const params: Record<string, string> = {};
  for (const [i, seg] of pattern.entries()) {
    const resource = resources[i] ?? '';
    if (seg.startsWith(':')) params[seg.slice(1)] = decodeURIComponent(resource);
    else if (seg !== resource) return null;
  }
  return params;
}
