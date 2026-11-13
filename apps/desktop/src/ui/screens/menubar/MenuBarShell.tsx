/**
 * The menu bar panel's own window frame: live engine events, without the
 * main window's title bar, palette or dialogs. It follows the Mac's light or
 * dark appearance, not Hoista's Theme setting: the system draws the glass
 * behind it to match the menu bar, so the text has to match too.
 */
import { useEffect } from 'react';
import { useEngineEvents } from '@ui/api/engine';
import { MenuBarPanel } from './MenuBarPanel';

export function MenuBarShell() {
  useEngineEvents();

  useEffect(() => {
    document.documentElement.classList.add('menubar');
    return () => {
      document.documentElement.classList.remove('menubar');
    };
  }, []);

  return <MenuBarPanel />;
}
