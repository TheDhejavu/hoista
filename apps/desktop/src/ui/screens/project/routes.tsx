/** Routes under `/projects/:appId`: one per tab, plus each resource's page. */
import type { RouteObject } from 'react-router-dom';
import { lazyScreen } from '@ui/lib/lazyScreen';

export const projectTabRoutes: RouteObject[] = [
  { index: true, lazy: lazyScreen(() => import('./overview/ProjectOverview'), 'ProjectOverview') },
  { path: 'map', lazy: lazyScreen(() => import('./map/MapTab'), 'MapTab') },
  { path: 'deploys', lazy: lazyScreen(() => import('./deploys/DeploysTab'), 'DeploysTab') },
  { path: 'logs', lazy: lazyScreen(() => import('./logs/LogsTab'), 'LogsTab') },
  { path: 'metrics', lazy: lazyScreen(() => import('./metrics/MetricsTab'), 'MetricsTab') },
  { path: 'env', lazy: lazyScreen(() => import('./env/EnvTab'), 'EnvTab') },
  { path: 'domains', lazy: lazyScreen(() => import('./domains/DomainsTab'), 'DomainsTab') },
  { path: 'settings', lazy: lazyScreen(() => import('./settings/SettingsTab'), 'SettingsTab') },
  { path: 'provider', lazy: lazyScreen(() => import('./provider/ProviderTab'), 'ProviderTab') },
  {
    path: 'activity',
    lazy: lazyScreen(() => import('./overview/ActivityPage'), 'ActivityPage'),
  },
  {
    path: 'resources/:resourceId',
    lazy: lazyScreen(() => import('./resources/ResourceScreen'), 'ResourceScreen'),
  },
  {
    path: 'resources/:resourceId/:tab',
    lazy: lazyScreen(() => import('./resources/ResourceScreen'), 'ResourceScreen'),
  },
];
