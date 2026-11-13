/**
 * The engine as the screens see it. Rust carries every request: to the
 * Hoista the screens show (this computer's, or a remote), or to this
 * computer's whichever is shown. Replies are read here, the same way for both.
 */
import { Channel, invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { EngineEvent, EngineRequest, EngineResult } from '@shared/api';
import type { Engine } from '@shared/engine';
import { messageData, parseServerSentEvent, toResult } from '@shared/wire';
import { asFailure } from './failure';

interface Reply {
  status: number;
  text: string;
}

type TailMessage = { kind: 'message'; text: string } | { kind: 'end' };

/** The Hoista the screens show. */
export function shownEngine(): Engine {
  return {
    request: (request) => send('engine_request', request),
    subscribe: followEvents,
    stream: tail,
  };
}

/** This computer's Hoista. Only asked directly; its events come through {@link shownEngine}. */
export function localEngine(): Engine {
  return {
    request: (request) => send('local_request', request),
    subscribe: () => () => undefined,
  };
}

async function send(command: string, request: EngineRequest): Promise<EngineResult> {
  try {
    const reply = await invoke<Reply>(command, { request });
    return toResult(reply.status, reply.text);
  } catch (failure) {
    return { ok: false, error: asFailure(failure) };
  }
}

/** Rust follows the shown Hoista's events and passes each one to every window. */
function followEvents(listener: (event: EngineEvent) => void): () => void {
  let stopped = false;
  let unlisten: (() => void) | null = null;
  void listen<string>('engine-event', ({ payload }) => {
    const event = parseServerSentEvent(payload);
    if (event) listener(event);
  }).then((stop) => {
    if (stopped) stop();
    else unlisten = stop;
  });
  return () => {
    stopped = true;
    unlisten?.();
  };
}

/** A live stream such as a log tail. Each frame's JSON goes to `onData`; `onEnd` runs once. */
function tail(path: string, onData: (data: unknown) => void, onEnd: () => void): () => void {
  let ended = false;
  let id: number | null = null;
  const end = (): void => {
    if (ended) return;
    ended = true;
    onEnd();
  };
  const channel = new Channel<TailMessage>();
  channel.onmessage = (message) => {
    if (message.kind === 'end') {
      end();
      return;
    }
    const data = messageData(message.text);
    if (!data) return;
    try {
      onData(JSON.parse(data) as unknown);
    } catch {
      // A malformed frame is skipped, not fatal.
    }
  };
  invoke<number>('tail_start', { path, onMessage: channel })
    .then((opened) => {
      id = opened;
      if (ended) void invoke('tail_stop', { id });
    })
    .catch(end);
  return () => {
    if (id !== null) void invoke('tail_stop', { id });
    end();
  };
}
