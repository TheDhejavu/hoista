/**
 * What the main window does besides its screens: keep the menu bar icon and
 * Dock badge up to date, show notifications, and ask Rust for the
 * right-click menu. It runs only in the main window, which stays open
 * (hidden) for as long as the app runs.
 */
import { invoke } from '@tauri-apps/api/core';
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from '@tauri-apps/plugin-notification';
import type { Engine } from '@shared/engine';
import { watchForNotices, type Note } from './notices';
import { iconState, readStatus, waitingCount } from './status';

/** Engine events trigger a refresh; this catches anything else, such as an approval expiring. */
const REFRESH_EVERY_MS = 30_000;
/** Events often come in bursts; the icon is read once per burst. */
const SETTLE_MS = 300;

export function startMainWindow(engine: Engine, tray: boolean): void {
  if (tray) keepTrayStatus(engine);
  watchForNotices(engine, { show: notify });
  window.addEventListener('contextmenu', showContextMenu);
}

function keepTrayStatus(engine: Engine): void {
  let pending: ReturnType<typeof setTimeout> | undefined;
  const refresh = async (): Promise<void> => {
    const status = await readStatus(engine);
    await invoke('tray_show', { state: iconState(status), waiting: waitingCount(status) });
  };
  const soon = (): void => {
    clearTimeout(pending);
    pending = setTimeout(() => void refresh().catch(() => undefined), SETTLE_MS);
  };
  engine.subscribe(soon);
  setInterval(soon, REFRESH_EVERY_MS);
  soon();
}

let allowed: Promise<boolean> | null = null;

/** Skipped while the window is in front: the app already shows it there. */
function notify(note: Note): void {
  if (document.hasFocus()) return;
  allowed ??= (async () =>
    (await isPermissionGranted()) || (await requestPermission()) === 'granted')().catch(
    () => false,
  );
  void allowed.then((ok) => {
    if (ok) sendNotification({ title: note.title, body: note.body });
  });
}

function showContextMenu(event: MouseEvent): void {
  // In development, Alt-right-click keeps the webview's own menu, with Inspect.
  if (import.meta.env.DEV && event.altKey) return;
  event.preventDefault();
  const target = event.target instanceof Element ? event.target : null;
  const field = target?.closest('input, textarea, [contenteditable]');
  const editable =
    field instanceof HTMLTextAreaElement ||
    (field instanceof HTMLInputElement && !['checkbox', 'radio', 'button'].includes(field.type)) ||
    (field instanceof HTMLElement && field.isContentEditable);
  const clicked = {
    link: target?.closest('a[href]')?.getAttribute('href') ?? null,
    editable,
    selected: (window.getSelection()?.toString().trim() ?? '') !== '',
  };
  void invoke('context_menu', { clicked });
}
