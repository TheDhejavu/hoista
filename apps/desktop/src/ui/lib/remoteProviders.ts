/**
 * Names for where Hoista runs when it isn't only on this computer: one of
 * the person's provider accounts (whichever providers the engine can run it on), or
 * a server of their own. Each provider bills the person directly; Hoista shows
 * no estimate.
 */
import type { RemoteTarget } from '@shared/api';
import { serviceName } from '@shared/providers';

/** The provider's name, "your remote", or "your cloud" when nothing is chosen yet. */
export function remotePlace(target: RemoteTarget): string {
  if (target === 'server') return 'your remote';
  if (target === 'this_computer') return 'your cloud';
  return serviceName(target);
}
