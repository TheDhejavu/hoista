/**
 * A resource's own tabs, under its name: Overview, then the Deploys, Logs, Metrics,
 * Variables its provider has for it, then Settings. Each tab is the project's
 * tab narrowed to this one resource; the project's tabs still show them all.
 */
import { NavLink } from 'react-router-dom';
import type { Resource } from '@shared/api';
import { providerEntry } from '@shared/providers';
import { useCan } from '@ui/api/providers';
import { cx } from '@ui/lib/cx';
import { DeploysTab } from '../deploys/DeploysTab';
import { EnvTab } from '../env/EnvTab';
import { LogsTab } from '../logs/LogsTab';
import { MetricsTab } from '../metrics/MetricsTab';
import { InResourcePage } from '../ProjectTab';
import { runsCode } from '../resourceKinds';
import { ResourceSettings } from './ResourceSettings';
import layout from '../ProjectLayout.module.css';

export type ResourceTab = 'overview' | 'deploys' | 'logs' | 'metrics' | 'env' | 'settings';

const LABELS: Record<ResourceTab, string> = {
  overview: 'Overview',
  deploys: 'Deploys',
  logs: 'Logs',
  metrics: 'Metrics',
  env: 'Variables',
  settings: 'Settings',
};

/** The tabs this resource has: only what its provider keeps for its kind. */
function useResourceTabs(resource: Resource): ResourceTab[] {
  const can = useCan();
  const entry = providerEntry(resource.provider);
  const code = runsCode(resource.kind);
  const logs =
    code ||
    (resource.kind === 'database' && entry?.databaseLogs === true) ||
    (entry?.logKinds?.includes(resource.kind) ?? false);
  const metrics =
    can(resource.provider, 'metrics') &&
    (entry?.charts?.refs ?? ['']).some((ref) => resource.provider_ref.startsWith(ref));
  const env = code || (entry?.variables?.on?.includes(resource.kind) ?? false);
  return [
    'overview',
    ...(code ? (['deploys'] as const) : []),
    ...(logs ? (['logs'] as const) : []),
    ...(metrics ? (['metrics'] as const) : []),
    ...(env ? (['env'] as const) : []),
    'settings',
  ];
}

export function isResourceTab(value: string | undefined): value is ResourceTab {
  return value !== undefined && value in LABELS;
}

export function ResourceTabs({ projectId, resource }: { projectId: string; resource: Resource }) {
  const tabs = useResourceTabs(resource);
  const base = `/projects/${projectId}/resources/${resource.id}`;
  return (
    <nav aria-label={`${resource.name} sections`} className={cx(layout.tabs, layout.resourceTabs)}>
      {tabs.map((tab) => (
        <NavLink
          key={tab}
          to={tab === 'overview' ? base : `${base}/${tab}`}
          end={tab === 'overview'}
          className={({ isActive }) => cx(layout.tab, isActive && layout.activeTab)}
        >
          {LABELS[tab]}
        </NavLink>
      ))}
    </nav>
  );
}

/** A tab other than Overview, narrowed to this resource. */
export function ResourceTabBody({
  tab,
  projectId,
  resource,
}: {
  tab: Exclude<ResourceTab, 'overview'>;
  projectId: string;
  resource: Resource;
}) {
  return (
    <InResourcePage.Provider value>
      {tab === 'deploys' && <DeploysTab pinned={resource.id} />}
      {tab === 'logs' && <LogsTab pinned={resource.id} />}
      {tab === 'metrics' && <MetricsTab pinned={resource.id} />}
      {tab === 'env' && <EnvTab pinned={resource.id} />}
      {tab === 'settings' && <ResourceSettings projectId={projectId} resource={resource} />}
    </InResourcePage.Provider>
  );
}
