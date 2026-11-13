/**
 * The bridge to the engine: `window.hoista`, which the desktop app sets
 * before the screens start (see `src/native/bridge.ts`).
 */
import type { HoistaBridge } from '@shared/bridge';

export function getBridge(): HoistaBridge {
  return window.hoista;
}
