/** Routes for the agents screens. Approvals happen in the agent itself. */
import type { RouteObject } from 'react-router-dom';
import { lazyScreen } from '@ui/lib/lazyScreen';

export const agentRoutes: RouteObject[] = [
  { path: 'agents', lazy: lazyScreen(() => import('./AgentsScreen'), 'AgentsScreen') },
  {
    path: 'agents/activity',
    lazy: lazyScreen(() => import('./ActivityScreen'), 'ActivityScreen'),
  },
  {
    path: 'reconnect/:service',
    lazy: lazyScreen(() => import('./ReconnectScreen'), 'ReconnectScreen'),
  },
];
