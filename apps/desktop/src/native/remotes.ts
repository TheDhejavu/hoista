/**
 * The `/remotes` routes, answered by Rust, which keeps the list, the keys
 * and which Hoista is shown. The screens call them like any other route.
 */
import { invoke } from '@tauri-apps/api/core';
import type { RemoteContents, RemotesView, ServerMove } from '@shared/api';
import type { Engine } from '@shared/engine';
import { RouteError, Router } from '@shared/router';
import { asFailure } from './failure';

async function call<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    const failure = asFailure(error);
    throw new RouteError(failure.code, failure.message);
  }
}

function text(body: unknown, field: string): string {
  const value = (body as Record<string, unknown> | undefined)?.[field];
  return typeof value === 'string' ? value : '';
}

export function withRemotes(engine: Engine): Engine {
  const router = new Router();
  router.on('GET', '/remotes', () => call<RemotesView>('remotes_view'));
  router.on('POST', '/remotes', ({ body }) =>
    call<RemotesView>('remote_add', { url: text(body, 'url'), name: text(body, 'name') || null }),
  );
  router.on('PUT', '/remotes/showing', ({ body }) =>
    call<RemotesView>('remote_show', { showing: text(body, 'showing') }),
  );
  router.on('PATCH', '/remotes/:id', ({ params, body }) =>
    call<RemotesView>('remote_rename', { id: params.id, name: text(body, 'name') }),
  );
  router.on('DELETE', '/remotes/:id', ({ params }) =>
    call<RemotesView>('remote_forget', { id: params.id }),
  );
  router.on('POST', '/remotes/:id/sign-in', ({ params }) =>
    call<RemotesView>('remote_sign_in', { id: params.id }),
  );
  router.on('DELETE', '/remotes/:id/sign-in', ({ params }) =>
    call<RemotesView>('remote_sign_out', { id: params.id }),
  );
  router.on('GET', '/remotes/:id/contents', ({ params }) =>
    call<RemoteContents>('remote_contents', { id: params.id }),
  );
  router.on('POST', '/remotes/:id/push', ({ params }) =>
    call<ServerMove>('remote_push', { id: params.id }),
  );

  return {
    request: (request) =>
      router.handles(request) ? router.dispatch(request, () => undefined) : engine.request(request),
    subscribe: (listener) => engine.subscribe(listener),
    ...(engine.stream ? { stream: engine.stream.bind(engine) } : {}),
  };
}
