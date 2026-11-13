/**
 * The app's keyboard shortcuts, answered by the window itself so they work
 * the same in the app and in a browser preview: ⌘K (Ctrl+K on Linux) to
 * search, ⌘1–5 for the sections, ⌘[ and ⌘] for back and forward, ⌘R to
 * refresh, ⌘, for Settings and ⌘/ for the full list. The app menu shows
 * the same ones (see `@shared/shortcuts`).
 */
import { useEffect } from 'react';
import { commandForKey } from '@shared/shortcuts';
import { useAppCommand } from './useAppCommand';

export function useShortcuts(): void {
  const run = useAppCommand();

  useEffect(() => {
    function onKeyDown(event: globalThis.KeyboardEvent): void {
      if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) return;
      const command = commandForKey(event.key);
      if (!command) return;
      // In a text field, ⌘[ and ⌘] belong to the field.
      const typing =
        event.target instanceof HTMLElement &&
        (event.target.isContentEditable || ['INPUT', 'TEXTAREA'].includes(event.target.tagName));
      if (typing && (command === 'back' || command === 'forward')) return;
      event.preventDefault();
      run(command);
    }

    window.addEventListener('keydown', onKeyDown);
    return () => {
      window.removeEventListener('keydown', onKeyDown);
    };
  }, [run]);
}
