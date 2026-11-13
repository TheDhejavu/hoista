/** The Billing screen. */
import type { RouteObject } from 'react-router-dom';
import { lazyScreen } from '@ui/lib/lazyScreen';

export const billingRoutes: RouteObject[] = [
  { path: 'billing', lazy: lazyScreen(() => import('./BillingScreen'), 'BillingScreen') },
];
