/**
 * Where a paused agent request sends you: the reconnect page for the provider
 * it waits on, saying which agent, project and action are waiting.
 */
import type { WaitingOnAccount } from './api';

export function reconnectPath(waiting: WaitingOnAccount): string {
  const params = new URLSearchParams({ agent: waiting.agent, action: waiting.action });
  if (waiting.project_id) params.set('project', waiting.project_id);
  return `/reconnect/${waiting.service}?${params.toString()}`;
}
