/**
 * Renders the screens: fonts, global styles, data cache and the router. The UI
 * uses the system face (SF Pro on macOS); Inter is bundled as the stand-in
 * elsewhere, and Geist Mono for addresses, numbers and logs.
 */
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { MutationCache, QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider } from 'react-router-dom';
import '@fontsource/inter/400.css';
import '@fontsource/inter/500.css';
import '@fontsource/inter/600.css';
import '@fontsource/inter/700.css';
import '@fontsource/geist-mono/400.css';
import '@fontsource/geist-mono/500.css';
import './styles/tokens.css';
import './styles/base.css';
import { getBridge } from './api/bridge';
import { OverlayProvider } from './layout/useOverlays';
import { PaletteProvider } from './palette/usePalette';
import { preloadScreens } from './lib/lazyScreen';
import { keepCache, restoreCache } from './lib/persistCache';
import { followSwitches } from './lib/switching';
import { showToast } from './lib/toast';
import { router } from './routes';

// Engine data changes through live events, so refetching on window focus is not needed.
// A mutation with `meta.toast` confirms itself with a short note once it succeeds.
const queryClient = new QueryClient({
  defaultOptions: { queries: { refetchOnWindowFocus: false, retry: 1 } },
  mutationCache: new MutationCache({
    onSuccess: (data, variables, _context, mutation) => {
      const toast = mutation.meta?.toast;
      if (toast) showToast(typeof toast === 'string' ? toast : toast(data, variables));
    },
  }),
});

// Opens on what the app saw last time, then refreshes it in the background.
restoreCache(queryClient);
keepCache(queryClient);

// Lets CSS adapt to the platform, for example the translucent sidebar on macOS.
document.documentElement.dataset.os = getBridge().platform;
// Mouse or keyboard, so focus rings show only for the keyboard (see base.css).
window.addEventListener(
  'pointerdown',
  () => {
    document.documentElement.dataset.input = 'pointer';
  },
  true,
);
window.addEventListener(
  'keydown',
  (event) => {
    if (event.key === 'Tab' || event.key.startsWith('Arrow')) {
      document.documentElement.dataset.input = 'keyboard';
    }
  },
  true,
);

const root = document.getElementById('root');
if (!root) throw new Error('index.html is missing #root.');

createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <PaletteProvider>
        <OverlayProvider>
          <RouterProvider router={router} />
        </OverlayProvider>
      </PaletteProvider>
    </QueryClientProvider>
  </StrictMode>,
);

preloadScreens();
// After switching instance: say where the app is now; follow switches made
// in another Hoista window.
followSwitches(showToast);
