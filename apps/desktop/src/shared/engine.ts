/**
 * The contract every engine client meets: the app's own connection to
 * `hoistd`, the adapter on top of it, and the socket client tests use.
 */
import type { EngineEvent, EngineRequest, EngineResult } from './api';

/** Anything that can answer local API requests and stream events. */
export interface Engine {
  request(request: EngineRequest): Promise<EngineResult>;
  /** Calls `listener` for every live event. Returns a function that stops it. */
  subscribe(listener: (event: EngineEvent) => void): () => void;
  /**
   * Opens a server-sent stream such as a live log tail and calls `onData` with
   * each frame's JSON. `onEnd` runs when it closes. Returns a function that
   * closes it. Engines without streams leave this out and callers poll.
   */
  stream?(path: string, onData: (data: unknown) => void, onEnd: () => void): () => void;
}

/** Thrown by {@link unwrap} when the engine reports an error. */
export class EngineError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = 'EngineError';
    this.code = code;
  }
}

/**
 * The data from a successful result, or an {@link EngineError}. The caller
 * names the response type: results cross IPC as plain JSON, so this is the
 * one place their type is asserted.
 */
// eslint-disable-next-line @typescript-eslint/no-unnecessary-type-parameters -- the caller names T
export function unwrap<T>(result: EngineResult): T {
  if (!result.ok) throw new EngineError(result.error.code, result.error.message);
  return result.data as T;
}
