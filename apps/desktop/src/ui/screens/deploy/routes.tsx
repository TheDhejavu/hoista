/** Routes for deploying with your agent and following a deploy as it runs. */
import type { RouteObject } from 'react-router-dom';
import { lazyScreen } from '@ui/lib/lazyScreen';

export const deployRoutes: RouteObject[] = [
  {
    path: 'deploys',
    lazy: lazyScreen(() => import('./DeployWithAgentScreen'), 'DeployWithAgentScreen'),
  },
  { path: 'deploys/:deployId', lazy: lazyScreen(() => import('./DeployScreen'), 'DeployScreen') },
];
