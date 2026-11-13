/** What each kind of resource is called on screen. */
import type { ProviderId, ResourceKind } from '@shared/api';
import { providerEntry } from '@shared/providers';

const RESOURCE_KIND_LABELS: Record<ResourceKind, string> = {
  web: 'Web service',
  service: 'Service',
  job: 'Cron job',
  database: 'Database',
  bucket: 'Bucket',
  email: 'Email sender',
  domain: 'Domain',
};

export function resourceKindLabel(kind: ResourceKind): string {
  return RESOURCE_KIND_LABELS[kind];
}

/**
 * What the provider itself calls a resource, singular, such as `Worker` on
 * Cloudflare or `App` on Fly.io; the general kind where it has no word.
 */
export function resourceTerm(resource: {
  provider: ProviderId;
  kind: ResourceKind;
  provider_ref: string;
}): string {
  return (
    providerEntry(resource.provider)?.words?.term?.(resource.kind, resource.provider_ref) ??
    RESOURCE_KIND_LABELS[resource.kind]
  );
}

/**
 * Whether a resource runs the project's own code. A project needs one: a database,
 * storage, email sender or domain is something a project uses, not a project.
 */
export function runsCode(kind: ResourceKind): boolean {
  return kind === 'web' || kind === 'service' || kind === 'job';
}
