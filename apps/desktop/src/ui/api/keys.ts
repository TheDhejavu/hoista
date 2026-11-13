/**
 * Every React Query key, in one place.
 *
 * Keys are nested so one invalidation reaches everything below it: refreshing
 * `projectKeys.detail(id)` also refreshes that project's deploys, logs and
 * secrets, and refreshing `projectKeys.all` reaches every project and every
 * resource.
 */
import type { ProjectListQuery, LogQuery, ServiceId } from '@shared/api';

export const projectKeys = {
  all: ['projects'] as const,
  list: (query: ProjectListQuery) => [...projectKeys.all, 'list', query] as const,
  lists: () => [...projectKeys.all, 'list'] as const,
  detail: (id: string) => [...projectKeys.all, id] as const,
  map: (id: string) => [...projectKeys.detail(id), 'map'] as const,
  activity: (id: string) => [...projectKeys.detail(id), 'activity'] as const,
  deploys: (id: string) => [...projectKeys.detail(id), 'deploys'] as const,
  logs: (id: string, query: LogQuery) => [...projectKeys.detail(id), 'logs', query] as const,
  secrets: (id: string) => [...projectKeys.detail(id), 'secrets'] as const,
  domains: (id: string) => [...projectKeys.detail(id), 'domains'] as const,
  settings: (id: string) => [...projectKeys.detail(id), 'settings'] as const,
  resource: (resourceId: string) => [...projectKeys.all, 'resources', resourceId] as const,
  /** Resources found on connected providers that are not in a project yet. */
  found: ['projects', 'found'] as const,
  attention: ['attention'] as const,
};

/** Everything about one deploy sits under `detail(id)`, so live events refresh it all. */
export const deployKeys = {
  /** The latest deploys across every project. */
  recent: ['deploys'] as const,
  detail: (id: string) => ['deploys', id] as const,
  log: (id: string) => [...deployKeys.detail(id), 'log'] as const,
  summary: (id: string) => [...deployKeys.detail(id), 'summary'] as const,
};

export const accountKeys = {
  all: ['accounts'] as const,
  detail: (id: string) => [...accountKeys.all, id] as const,
  catalogue: ['catalogue'] as const,
  guide: (service: ServiceId) => [...accountKeys.catalogue, service, 'connect'] as const,
  profile: ['profile'] as const,
};

export const agentKeys = {
  all: ['agents'] as const,
  activity: (agentId: string | undefined) =>
    [...agentKeys.all, 'activity', agentId ?? 'all'] as const,
  requests: ['agents', 'requests'] as const,
  approvals: ['approvals'] as const,
  approval: (id: string) => [...agentKeys.approvals, id] as const,
};

/** What each resource's provider reports, by resource; any change at a provider refreshes it all. */
export const providerKeys = {
  all: ['provider'] as const,
  detail: (resourceId: string) => [...providerKeys.all, resourceId] as const,
};

export const settingsKeys = {
  all: ['settings'] as const,
};
