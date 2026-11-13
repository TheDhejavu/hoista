/** Routes for accounts. */
import type { RouteObject } from 'react-router-dom';
import { lazyScreen } from '@ui/lib/lazyScreen';

export const accountRoutes: RouteObject[] = [
  {
    path: 'accounts',
    lazy: lazyScreen(() => import('./AccountsScreen'), 'AccountsScreen'),
    children: [
      {
        path: 'connect/:serviceId',
        lazy: lazyScreen(() => import('./ConnectDialog'), 'ConnectDialog'),
      },
      {
        path: ':accountId',
        lazy: lazyScreen(() => import('./ManageAccountDialog'), 'ManageAccountDialog'),
      },
    ],
  },
];
