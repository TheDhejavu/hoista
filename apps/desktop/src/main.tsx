/**
 * Starts a window: asks Rust what the page needs to know, puts
 * `window.hoista` in place, then renders the screens. The panel under the
 * menu bar icon loads the same page and opens its own screen.
 */
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { createBridge, type Startup } from '@native/bridge';

const startup = await invoke<Startup>('startup');
const current = getCurrentWebviewWindow();
if (current.label === 'panel') window.location.hash = '/menubar';
else if (startup.start_at) window.location.hash = startup.start_at;

window.hoista = createBridge(startup);
await import('@ui/start');

// Shown once the page has painted, so the window never flashes the wrong colour.
if (current.label === 'main') {
  requestAnimationFrame(() => {
    requestAnimationFrame(() => void current.show());
  });
}
