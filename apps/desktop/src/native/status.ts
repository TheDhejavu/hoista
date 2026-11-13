/**
 * What the menu bar and notifications need to know, read from the engine:
 * what waits for the user, what's deploying, and how they want to be told.
 */
import type { AttentionItem, ConnectionRequest, Deploy, Page, Settings } from '@shared/api';
import { unwrap, type Engine } from '@shared/engine';

export interface Status {
  /** Coding tools asking to connect. */
  requests: ConnectionRequest[];
  /** Needs attention items for agent requests that wait on a provider. */
  waiting: AttentionItem[];
  /** Deploys running now. */
  deploying: Deploy[];
  /** False after Stop everything, when agents can't reach Hoista. */
  agentsCanConnect: boolean;
  notify: Settings['general']['notify'];
}

/** What the menu bar icon shows: stopped, something waiting, a deploy running, or all quiet. */
export type IconState = 'normal' | 'needs' | 'deploying' | 'stopped';

export function iconState(status: Status): IconState {
  if (!status.agentsCanConnect) return 'stopped';
  if (waitingCount(status) > 0) return 'needs';
  return status.deploying.length > 0 ? 'deploying' : 'normal';
}

/** How many things wait for the user: what the Dock badge counts. */
export function waitingCount(status: Status): number {
  return status.waiting.length + status.requests.length;
}

/** Reads the status. Resources the engine can't answer count as empty. */
export async function readStatus(engine: Engine): Promise<Status> {
  const [attention, deploys, settings, requests] = await Promise.all([
    get<Page<AttentionItem>>(engine, '/attention'),
    get<Page<Deploy>>(engine, '/deploys'),
    get<Settings>(engine, '/settings'),
    get<Page<ConnectionRequest>>(engine, '/agents/requests'),
  ]);
  return {
    requests: requests?.items ?? [],
    waiting: (attention?.items ?? []).filter((item) => item.waiting_on !== null),
    deploying: (deploys?.items ?? []).filter(
      (deploy) => deploy.status === 'running' || deploy.status === 'queued',
    ),
    agentsCanConnect: settings?.agents.connection_enabled ?? true,
    notify: settings?.general.notify ?? 'problems',
  };
}

/** The data from a GET, or null if the engine could not answer. */
export async function get<T>(engine: Engine, path: string): Promise<T | null> {
  try {
    return unwrap<T>(await engine.request({ method: 'GET', path }));
  } catch {
    return null;
  }
}
