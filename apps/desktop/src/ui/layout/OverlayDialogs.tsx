/**
 * Renders the open Settings, Get help or Keyboard shortcuts dialog, and closes it when the user
 * navigates somewhere else from inside it (for example "Manage agents").
 */
import { useEffect, useRef } from 'react';
import { useLocation } from 'react-router-dom';
import { HelpDialog } from '@ui/screens/settings/HelpDialog';
import { SettingsDialog } from '@ui/screens/settings/SettingsDialog';
import { ShortcutsDialog } from '@ui/screens/settings/ShortcutsDialog';
import { useOverlays } from './useOverlays';

export function OverlayDialogs() {
  const { overlay, close } = useOverlays();
  const { pathname } = useLocation();
  const lastPath = useRef(pathname);

  // Only a real move closes it, not the first render (a link may open the app with Settings showing).
  useEffect(() => {
    if (lastPath.current === pathname) return;
    lastPath.current = pathname;
    close();
  }, [pathname, close]);

  switch (overlay?.kind) {
    case 'settings':
      return <SettingsDialog initialSection={overlay.section} onClose={close} />;
    case 'help':
      return <HelpDialog onClose={close} />;
    case 'shortcuts':
      return <ShortcutsDialog onClose={close} />;
    case undefined:
      return null;
  }
}
