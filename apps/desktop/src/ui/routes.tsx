/**
 * Every screen in the app. Hash routing, because the packaged app loads
 * from a file and has no server to resolve paths. The Projects list and the
 * layouts load with the app; every other screen loads when first opened.
 */
import { createHashRouter } from 'react-router-dom';
import { AppShell } from './layout/AppShell';
import { FirstRunLayout } from './layout/FirstRunLayout';
import { SidebarLayout } from './layout/SidebarLayout';
import { accountRoutes } from './screens/accounts/routes';
import { firstRunRoutes } from './screens/firstrun/routes';
import { agentRoutes } from './screens/agents/routes';
import { billingRoutes } from './screens/billing/routes';
import { projectTabRoutes } from './screens/project/routes';
import { ProjectsScreen } from './screens/projects/ProjectsScreen';
import { deployRoutes } from './screens/deploy/routes';
import { guideRoutes } from './screens/guides/routes';
import { settingsRoutes } from './screens/settings/routes';
import { lazyScreen } from './lib/lazyScreen';
import { ScreenError } from './screens/ScreenError';
import { AppError } from './screens/AppError';

export const router = createHashRouter([
  // The menu bar panel, in its own small window.
  {
    path: 'menubar',
    lazy: lazyScreen(() => import('./screens/menubar/MenuBarShell'), 'MenuBarShell'),
  },
  {
    element: <AppShell />,
    // If the frame itself fails, the window says so and offers a reload.
    errorElement: <AppError />,
    // Screens load on first visit; until the first one is ready the window shows its background.
    hydrateFallbackElement: null,
    children: [
      { element: <FirstRunLayout />, children: firstRunRoutes },
      {
        element: <SidebarLayout />,
        children: [
          {
            // A screen that fails shows this inside the window; the sidebar keeps working.
            errorElement: <ScreenError />,
            children: [
              { index: true, element: <ProjectsScreen /> },
              {
                path: 'projects/:projectId',
                lazy: lazyScreen(() => import('./screens/project/ProjectLayout'), 'ProjectLayout'),
                children: projectTabRoutes,
              },
              ...deployRoutes,
              ...accountRoutes,
              ...billingRoutes,
              ...agentRoutes,
              ...settingsRoutes,
              ...guideRoutes,
              { path: '*', lazy: lazyScreen(() => import('./screens/NotFound'), 'NotFound') },
            ],
          },
        ],
      },
    ],
  },
]);
