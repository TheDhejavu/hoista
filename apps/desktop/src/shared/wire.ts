/**
 * Reading what an engine sends back: a reply's status and body, and the
 * messages of its event stream.
 */
import type { EngineEvent, EngineResult } from './api';

/** Turns an HTTP status and body into a result. Error bodies are `{ code, message }`. */
export function toResult(status: number, text: string): EngineResult {
  const json = parseJson(text);
  if (status < 400) return { ok: true, data: json };

  return {
    ok: false,
    error: {
      code: stringField(json, 'code') ?? `http_${status}`,
      message: stringField(json, 'message') ?? 'The Hoista engine returned an error.',
    },
  };
}

/** The result when the engine's socket doesn't answer. */
export function engineUnreachable(): EngineResult {
  return {
    ok: false,
    error: { code: 'engine_unreachable', message: 'The Hoista engine is not running.' },
  };
}

/** The joined `data:` lines of one server-sent message, or null when it has none. */
export function messageData(message: string): string | null {
  const data = message
    .split('\n')
    .filter((line) => line.startsWith('data:'))
    .map((line) => line.slice('data:'.length).trim())
    .join('\n');
  return data || null;
}

/**
 * Reads one server-sent event. Returns null for heartbeats, `stream.error`
 * frames and anything that is not an activity entry.
 */
export function parseServerSentEvent(message: string): EngineEvent | null {
  if (message.split('\n').some((line) => line.trim() === 'event: stream.error')) return null;
  const data = messageData(message);
  if (!data) return null;
  try {
    const value: unknown = JSON.parse(data);
    return isEngineEvent(value) ? value : null;
  } catch {
    return null;
  }
}

function isEngineEvent(value: unknown): value is EngineEvent {
  if (typeof value !== 'object' || value === null) return false;
  const event = value as Record<string, unknown>;
  return typeof event.id === 'string' && typeof event.kind === 'string';
}

function parseJson(text: string): unknown {
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

function stringField(json: unknown, key: string): string | null {
  if (typeof json !== 'object' || json === null) return null;
  const value: unknown = Reflect.get(json, key);
  return typeof value === 'string' ? value : null;
}
