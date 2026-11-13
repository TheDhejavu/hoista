/** Routes for the provider guides. */
import type { RouteObject } from 'react-router-dom';
import { lazyScreen } from '@ui/lib/lazyScreen';

export const guideRoutes: RouteObject[] = [
  { path: 'guides', lazy: lazyScreen(() => import('./GuidesScreen'), 'GuidesScreen') },
  {
    path: 'guides/:provider/:step',
    lazy: lazyScreen(() => import('./ProviderGuideScreen'), 'ProviderGuideScreen'),
  },
];
