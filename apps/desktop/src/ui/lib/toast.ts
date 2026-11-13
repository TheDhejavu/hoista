/**
 * Short notes about something you just did, such as "Saved", the way Linear,
 * Figma and Notion show them: at the bottom of the window, gone after a few
 * seconds. Anything can show one; the Toasts component in the window frame
 * draws them.
 *
 * A note has a kind (done, a problem, or just so you know) and can carry one
 * action, such as View or Undo. A note with a `detail` line is drawn as a
 * small card in the corner instead, for a summary worth reading, such as
 * "Hoista is set up". Hovering over the notes keeps them up;
 * problems stay longer. Things that happen while you're away, such as a
 * deploy finishing, go to the system's notifications instead (main/notifications).
 */

export type ToastTone = 'success' | 'error' | 'info';

export interface ToastAction {
  label: string;
  onClick: () => void;
}

export interface Toast {
  id: number;
  text: string;
  tone: ToastTone;
  action?: ToastAction;
  /** A second line under the text: the note becomes a card, kept up longer. */
  detail?: string;
  /** Fading out, just before it's removed. */
  leaving?: boolean;
}

interface ToastOptions {
  tone?: ToastTone;
  action?: ToastAction;
  detail?: string;
}

/** How long a note stays before it fades; problems stay longer. */
const TOAST_MS = 2600;
const ERROR_TOAST_MS = 6000;
/** At most this many at once; older ones make way. */
const MAX_SHOWN = 3;

let shown: Toast[] = [];
let nextId = 1;
let paused = false;
const timers = new Map<
  number,
  { left: number; started: number; handle?: ReturnType<typeof setTimeout> }
>();
const listeners = new Set<(toasts: Toast[]) => void>();

function publish(): void {
  for (const listener of listeners) listener(shown);
}

/** How long a note takes to fade out. */
const LEAVE_MS = 180;
/** A paused toast keeps at least this long once it resumes, so it can still be read. */
const MIN_LEFT_MS = 800;

export function dismissToast(id: number): void {
  const timer = timers.get(id);
  if (timer?.handle) clearTimeout(timer.handle);
  timers.delete(id);
  if (!shown.some((item) => item.id === id && !item.leaving)) return;
  shown = shown.map((item) => (item.id === id ? { ...item, leaving: true } : item));
  publish();
  setTimeout(() => {
    shown = shown.filter((item) => item.id !== id);
    publish();
  }, LEAVE_MS);
}

function schedule(id: number): void {
  const timer = timers.get(id);
  if (!timer || paused) return;
  timer.started = Date.now();
  timer.handle = setTimeout(() => {
    dismissToast(id);
  }, timer.left);
}

export function showToast(text: string, options: ToastOptions = {}): void {
  const toast: Toast = {
    id: nextId++,
    text,
    tone: options.tone ?? 'success',
    ...(options.action ? { action: options.action } : {}),
    ...(options.detail ? { detail: options.detail } : {}),
  };
  shown = [...shown, toast].slice(-MAX_SHOWN);
  for (const id of [...timers.keys()]) {
    if (!shown.some((item) => item.id === id)) dismissToast(id);
  }
  timers.set(toast.id, {
    left: toast.tone === 'error' || toast.action || toast.detail ? ERROR_TOAST_MS : TOAST_MS,
    started: Date.now(),
  });
  schedule(toast.id);
  publish();
}

/** Keeps every note up while the pointer is over them. */
export function pauseToasts(): void {
  if (paused) return;
  paused = true;
  for (const timer of timers.values()) {
    if (timer.handle) clearTimeout(timer.handle);
    timer.handle = undefined;
    timer.left = Math.max(MIN_LEFT_MS, timer.left - (Date.now() - timer.started));
  }
}

/** Lets them go again, each with the time it had left. */
export function resumeToasts(): void {
  if (!paused) return;
  paused = false;
  for (const id of timers.keys()) schedule(id);
}

export function subscribeToToasts(listener: (toasts: Toast[]) => void): () => void {
  listeners.add(listener);
  listener(shown);
  return () => {
    listeners.delete(listener);
  };
}
