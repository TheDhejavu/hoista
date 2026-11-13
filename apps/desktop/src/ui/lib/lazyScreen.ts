/**
 * Loads a screen's code the first time someone opens it, so the window
 * appears quickly. The router keeps the current screen up while the next one
 * loads, so there is no blank flash.
 *
 * Every screen is also warmed in the background once the app is idle (see
 * `preloadScreens`), so later navigation is instant.
 */
import type { ComponentType } from 'react';
import type { RouteObject } from 'react-router-dom';
import { SECOND_MS } from '@shared/time';

/** Screens load while the app is idle, or after this long at the latest. */
const PRELOAD_WITHIN_MS = 3 * SECOND_MS;

type Loader = () => Promise<unknown>;

const loaders: Loader[] = [];

/**
 * `lazy: lazyScreen(() => import('./MapTab'), 'MapTab')` for a route whose
 * screen is the named export `MapTab`.
 */
export function lazyScreen<Module>(
  load: () => Promise<Module>,
  name: keyof Module,
): NonNullable<RouteObject['lazy']> {
  loaders.push(load);
  return async () => {
    const module = await load();
    return { Component: module[name] as ComponentType };
  };
}

/** Fetches every screen's code while nothing else is happening. */
export function preloadScreens(): void {
  const warm = (): void => {
    for (const load of loaders) void load();
  };
  window.requestIdleCallback(warm, { timeout: PRELOAD_WITHIN_MS });
}
