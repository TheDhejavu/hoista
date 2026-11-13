/**
 * Hoista on a server: put it on whichever provider account the person picks (any
 * provider whose adapter can run it), tell when it answers, and forget it
 * here. The engine does the work and keeps the record; this only checks
 * that the new server is up.
 */
import type { Settings as DaemonSettings } from '@engine-api';
import type { ServerProviders, ServerRequest, ServerSetup } from '@shared/api';
import { RouteError, type Router } from '@shared/router';
import type { Call } from './call';

export function addServerRoutes(
  router: Router,
  call: Call,
  answers: (url: string) => Promise<boolean>,
): void {
  // The providers Hoista can run on, as the engine's adapters say.
  router.on('GET', '/server/providers', async (): Promise<ServerProviders> => {
    const providers = await call<{
      items: { provider: string; capabilities: { server?: boolean } }[];
    }>('GET', '/providers');
    return {
      providers: providers.items
        .filter((item) => item.capabilities.server === true)
        .map((item) => item.provider as ServerProviders['providers'][number]),
    };
  });

  router.on('POST', '/server', async ({ body }): Promise<ServerSetup> => {
    const input = (body ?? {}) as Partial<ServerRequest>;
    if (!input.account_id) {
      throw new RouteError('server.invalid', 'Pick the account to run it in.');
    }
    // The published image, pinned by digest; the CLI can still build from a repository.
    return call<ServerSetup>('POST', '/server', { account_id: input.account_id });
  });

  // A Hoista already on a server (set up there with `hoist`): its address,
  // checked that it answers, then this computer signs in as with any remote.
  router.on('POST', '/server/connect', async ({ body }): Promise<null> => {
    const given = (body as { url?: unknown } | undefined)?.url;
    const typed = typeof given === 'string' ? given.trim() : '';
    // No scheme typed: this computer is plain http, anywhere else https.
    const withScheme = /^[a-z]+:\/\//i.test(typed)
      ? typed
      : /^(localhost|127\.0\.0\.1|\[::1\])(:|$)/.test(typed)
        ? `http://${typed}`
        : `https://${typed}`;
    let url: URL;
    try {
      url = new URL(withScheme);
    } catch {
      throw new RouteError(
        'server.invalid',
        'Give your remote’s address, such as https://hoista.example.com.',
      );
    }
    // Plain http only to this computer: a Hoista in Docker here.
    if (url.protocol !== 'https:' && !(url.protocol === 'http:' && isLoopback(url))) {
      throw new RouteError(
        'server.invalid',
        'Your remote’s address starts with https://, or http://localhost for one on this computer.',
      );
    }
    const origin = url.origin;
    if (!(await answers(origin))) {
      throw new RouteError(
        'server.unreachable',
        `Nothing answered at ${url.host}. Check the address, and that your remote is running.`,
      );
    }
    await call('POST', '/server/connect', { url: origin });
    return null;
  });

  router.on('GET', '/server/ready', async (): Promise<{ ready: boolean }> => {
    const settings = await call<DaemonSettings>('GET', '/settings');
    const url = settings.server?.url;
    if (!url) return { ready: false };
    const parsed = new URL(url);
    if (parsed.protocol !== 'https:' && !isLoopback(parsed)) return { ready: false };
    return { ready: await answers(url) };
  });

  router.on('DELETE', '/server', async (): Promise<null> => {
    await call('DELETE', '/server');
    return null;
  });
}

/** This computer: `localhost`, `127.0.0.1` or `[::1]`, which browsers treat as secure. */
function isLoopback(url: URL): boolean {
  return ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname);
}
