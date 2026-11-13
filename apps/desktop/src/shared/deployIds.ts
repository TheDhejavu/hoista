/**
 * Ids for deploys a provider made rather than Hoista:
 * `provider~{resource id}~{provider deployment id}`. The adapter gives these
 * ids out, and the screens read them back.
 */
const PREFIX = 'provider~';
const SEPARATOR = '~';

export function providerDeployId(resourceId: string, deploymentId: string): string {
  return `${PREFIX}${resourceId}${SEPARATOR}${deploymentId}`;
}

export function isProviderDeploy(id: string): boolean {
  return id.startsWith(PREFIX);
}

/** The resource a provider deploy belongs to and the provider's own id, or null for Hoista's. */
export function parseProviderDeploy(
  id: string,
): { resourceId: string; deploymentId: string } | null {
  if (!isProviderDeploy(id)) return null;
  const rest = id.slice(PREFIX.length);
  const split = rest.indexOf(SEPARATOR);
  if (split <= 0) return null;
  return { resourceId: rest.slice(0, split), deploymentId: rest.slice(split + 1) };
}
