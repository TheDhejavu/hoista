/**
 * Switching instance (this Mac, or one of your remotes), the way workspace
 * switchers in Slack or Linear do it: a full-window "Switching to …" screen,
 * then the whole app reloads on its home screen, so nothing from the
 * instance before stays on screen or in memory. Other Hoista windows (the
 * menu bar panel) reload too. After the reload a note says where you are.
 */
import { getBridge } from '@ui/api/bridge';
import { forgetCache } from './persistCache';

const NOTE = 'hoista.switched';

/** How long before the cover offers a way back. */
const SLOW_MS = 4_000;

/**
 * Covers the window while the switch happens, so stale screens never show.
 * If it's slow, it says so and offers to stay where you are (`onStay`).
 * Returns a function that takes the cover away.
 */
export function coverWindow(name: string, onStay?: () => void): () => void {
  const cover = document.createElement('div');
  cover.setAttribute('role', 'status');
  cover.setAttribute('aria-live', 'assertive');
  cover.style.cssText = [
    'position:fixed',
    'inset:0',
    'z-index:2147483647',
    'display:flex',
    'flex-direction:column',
    'align-items:center',
    'justify-content:center',
    'gap:14px',
    'background:var(--bg, #fff)',
    'color:var(--text, #111)',
    'font:600 15px/1.4 Inter, -apple-system, sans-serif',
    'opacity:0',
    'transition:opacity 120ms ease-out',
  ].join(';');
  const spinner = document.createElement('span');
  spinner.className = 'hoista-switch-spinner';
  spinner.style.cssText =
    'width:22px;height:22px;border-radius:50%;border:2.5px solid var(--line-strong, #ddd);border-top-color:var(--text, #111);animation:hoista-switch-spin .8s linear infinite';
  const style = document.createElement('style');
  style.textContent = [
    '@keyframes hoista-switch-spin{to{transform:rotate(360deg)}}',
    '@media (prefers-reduced-motion:reduce){.hoista-switch-spinner{animation:none!important}}',
    '.hoista-switch-note{margin:-6px 0 0;font-weight:400;font-size:13px;color:var(--text-2, #666)}',
    '.hoista-switch-stay{margin-top:4px;height:32px;padding:0 14px;border-radius:8px;border:1px solid var(--line-strong, #ddd);background:var(--bg-raised, #fff);color:var(--text, #111);font:500 13px Inter, -apple-system, sans-serif;cursor:pointer}',
    '.hoista-switch-stay:hover{background:var(--bg-muted, #f4f4f4)}',
    '.hoista-switch-stay:focus-visible{outline:2px solid var(--accent, #2563eb);outline-offset:2px}',
  ].join('');
  const text = document.createElement('span');
  text.textContent = `Switching to ${name}`;
  cover.append(style, spinner, text);
  document.body.append(cover);
  requestAnimationFrame(() => {
    cover.style.opacity = '1';
  });
  const slow = onStay
    ? setTimeout(() => {
        const note = document.createElement('p');
        note.className = 'hoista-switch-note';
        note.textContent = `Still waiting for ${name} to answer.`;
        const stay = document.createElement('button');
        stay.type = 'button';
        stay.className = 'hoista-switch-stay';
        stay.textContent = 'Stay where I am';
        stay.addEventListener('click', onStay);
        cover.append(note, stay);
        stay.focus();
      }, SLOW_MS)
    : undefined;
  return () => {
    clearTimeout(slow);
    cover.remove();
  };
}

/** Reloads this window on its home screen, and the other Hoista windows too. */
export function reloadInto(name: string): void {
  // The saved screens belong to the instance being left.
  forgetCache();
  try {
    sessionStorage.setItem(NOTE, name);
  } catch {
    // No note after the reload; the switch itself still happens.
  }
  window.location.hash = '#/';
  // The app does the reload: a page can't, since the windows refuse
  // navigation they didn't start. If even that fails, try it here.
  getBridge()
    .reloadWindows()
    .catch(() => {
      window.location.reload();
    });
}

/** At start: say which instance the app now shows, if it just switched. */
export function followSwitches(say: (text: string, options: { tone: 'info' }) => void): void {
  try {
    const name = sessionStorage.getItem(NOTE);
    if (name) {
      sessionStorage.removeItem(NOTE);
      say(`Now on ${name}`, { tone: 'info' });
    }
  } catch {
    // Nothing to say.
  }
}
