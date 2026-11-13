/**
 * First run, before the sidebar: the welcome screen, then setup, one step
 * per address so Back works and each step can be opened on its own.
 */
import { Navigate, type RouteObject } from 'react-router-dom';
import { lazyScreen } from '@ui/lib/lazyScreen';

export const firstRunRoutes: RouteObject[] = [
  { path: 'welcome', lazy: lazyScreen(() => import('./WelcomeScreen'), 'WelcomeScreen') },
  {
    path: 'setup',
    lazy: lazyScreen(() => import('./SetupLayout'), 'SetupLayout'),
    children: [
      { index: true, element: <Navigate to="providers" replace /> },
      {
        path: 'providers',
        lazy: lazyScreen(() => import('./PickProvidersStep'), 'PickProvidersStep'),
      },
      {
        path: 'connect',
        lazy: lazyScreen(() => import('./ConnectProvidersStep'), 'ConnectProvidersStep'),
      },
      {
        path: 'projects',
        lazy: lazyScreen(() => import('./PickProjectsStep'), 'PickProjectsStep'),
      },
      { path: 'ai', lazy: lazyScreen(() => import('./ConnectAIStep'), 'ConnectAIStep') },
    ],
  },
];
