/**
 * A Node client for `hoistd`'s socket, for tests that run the adapter
 * against a real engine. The app itself talks to the engine from Rust.
 */
import { request as httpRequest, type IncomingMessage } from 'node:http';
import type { EngineEvent, EngineRequest, EngineResult } from '@shared/api';
import type { Engine } from '@shared/engine';
import { engineUnreachable, messageData, parseServerSentEvent, toResult } from '@shared/wire';

const RECONNECT_DELAY_MS = 2_000;

/** Creates an engine client for the socket at `socketPath`. */
export function createSocketEngine(socketPath: string): Engine {
  return {
    request: (request) => sendRequest(socketPath, request),
    subscribe: (listener) => streamEvents(socketPath, listener),
    stream: (path, onData, onEnd) => streamPath(socketPath, path, onData, onEnd),
  };
}

/** One server-sent stream, without reconnecting: the caller decides what a drop means. */
function streamPath(
  socketPath: string,
  path: string,
  onData: (data: unknown) => void,
  onEnd: () => void,
): () => void {
  let ended = false;
  const end = () => {
    if (ended) return;
    ended = true;
    onEnd();
  };
  const req = httpRequest({ socketPath, method: 'GET', path }, (res) => {
    if ((res.statusCode ?? 500) >= 400) {
      res.resume();
      end();
      return;
    }
    res.setEncoding('utf8');
    let buffer = '';
    res.on('data', (chunk: string) => {
      buffer += chunk;
      if (buffer.length > 1_000_000) {
        req.destroy();
        return;
      }
      const messages = buffer.split('\n\n');
      buffer = messages.pop() ?? '';
      for (const message of messages) {
        const data = messageData(message);
        if (!data) continue;
        try {
          onData(JSON.parse(data) as unknown);
        } catch {
          // A malformed frame is skipped, not fatal.
        }
      }
    });
    res.on('end', end);
    res.on('close', end);
  });
  req.on('error', end);
  req.end();
  return () => {
    req.destroy();
    end();
  };
}

function sendRequest(socketPath: string, request: EngineRequest): Promise<EngineResult> {
  const body = request.body === undefined ? undefined : JSON.stringify(request.body);

  return new Promise((resolve) => {
    const req = httpRequest(
      {
        socketPath,
        method: request.method,
        path: request.path,
        // Node sends a DELETE's body only with its length given.
        headers: body
          ? {
              'content-type': 'application/json',
              'content-length': Buffer.byteLength(body),
            }
          : {},
      },
      (res) => {
        readBody(res)
          .then((text) => {
            resolve(toResult(res.statusCode ?? 500, text));
          })
          .catch(() => {
            resolve(engineUnreachable());
          });
      },
    );
    req.on('error', () => {
      resolve(engineUnreachable());
    });
    req.end(body);
  });
}

function readBody(res: IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    let text = '';
    res.setEncoding('utf8');
    res.on('data', (chunk: string) => (text += chunk));
    res.on('end', () => {
      resolve(text);
    });
    res.on('error', reject);
  });
}

/**
 * Keeps one `GET /events` stream open until the returned function is called.
 *
 * The first connection starts at the newest event (`after=latest`), so opening
 * the app does not replay old activity. After a drop it resumes with
 * `Last-Event-ID`, so nothing committed in between is missed.
 */
function streamEvents(socketPath: string, listener: (event: EngineEvent) => void): () => void {
  let stopped = false;
  let retry: NodeJS.Timeout | undefined;
  let lastId: string | null = null;

  function connect(): void {
    const req = httpRequest(
      {
        socketPath,
        method: 'GET',
        path: lastId === null ? '/events?after=latest' : '/events',
        headers: lastId === null ? {} : { 'Last-Event-ID': lastId },
      },
      (res) => {
        res.setEncoding('utf8');
        let buffer = '';
        res.on('data', (chunk: string) => {
          buffer += chunk;
          const messages = buffer.split('\n\n');
          buffer = messages.pop() ?? '';
          for (const message of messages) {
            const event = parseServerSentEvent(message);
            if (!event) continue;
            lastId = event.id;
            listener(event);
          }
        });
        res.on('end', scheduleReconnect);
      },
    );
    req.on('error', scheduleReconnect);
    req.end();
  }

  function scheduleReconnect(): void {
    if (!stopped) retry = setTimeout(connect, RECONNECT_DELAY_MS);
  }

  connect();
  return () => {
    stopped = true;
    clearTimeout(retry);
  };
}
