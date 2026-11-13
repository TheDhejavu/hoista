/**
 * Wording for where Hoista keeps running when this computer is off. No
 * provider is required: the default is this computer only, and each other
 * choice is an account the person may already have.
 */
import type { RemoteTarget, ServerProviders, Settings } from '@shared/api';
import type { Option } from '@ui/components/Select';
import { remotePlace } from '@ui/lib/remoteProviders';

/**
 * Local is the default; then each provider you've connected that can run
 * Hoista, then your own server, which needs no provider. Providers you
 * haven't connected aren't offered: connecting one comes first. The current
 * place stays listed even if its provider is no longer connected.
 */
export function remoteChoices(
  connected: ServerProviders['providers'],
  current: RemoteTarget,
): Option<RemoteTarget>[] {
  const places: RemoteTarget[] = [...connected];
  if (current !== 'this_computer' && !places.includes(current)) places.push(current);
  if (!places.includes('server')) places.push('server');
  return [
    { value: 'this_computer', label: 'Only on this computer' },
    ...places.map((place) => ({
      value: place,
      label:
        place === 'server'
          ? 'Remote · your own server'
          : `Remote · ${remotePlace(place)} in your account`,
    })),
  ];
}

/** One line on what the choice means, shown under it. */
export function remoteHint(target: RemoteTarget): string {
  if (target === 'this_computer') {
    return 'Strictly local. Agents on this computer connect directly and nothing is reachable from the internet. Chat apps on the web, like ChatGPT, connect through MCP over HTTP with a tunnel, or through a remote.';
  }
  if (target === 'server') {
    return 'Hoista runs on your own server at its own address, so chat apps and cloud agents reach it when this computer is off. You sign in there with a passkey. Nothing runs on any server of ours.';
  }
  return `Hoista runs in your own ${remotePlace(target)} account at its own address, so chat apps and cloud agents reach it when this computer is off. You sign in there with a passkey. Nothing runs on any server of ours.`;
}

/**
 * The address chat apps and cloud agents add: the server's `/mcp` once Hoista is on one.
 * Null before that.
 */
export function agentUrl(agents: Settings['agents']): string | null {
  if (agents.remote.url) return `${agents.remote.url.replace(/\/$/, '')}/mcp`;
  return null;
}

/** Where a remote runs, in a few words: "Your own server", or the provider's name "in your account". */
export function remoteName(target: Exclude<RemoteTarget, 'this_computer'>): string {
  return target === 'server' ? 'Your own server' : `${remotePlace(target)} in your account`;
}
