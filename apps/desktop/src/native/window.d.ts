/** Types `window.hoista` for the screens. */
import type { HoistaBridge } from '@shared/bridge';

declare global {
  interface Window {
    /** Set by the desktop app before the screens start. */
    hoista: HoistaBridge;
  }
}

export {};
