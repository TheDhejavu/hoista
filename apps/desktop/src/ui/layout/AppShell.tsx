/**
 * The window frame around every screen: title bar, the refresh bar, live engine events,
 * keyboard shortcuts, the command palette, the Settings and Help dialogs,
 * agents asking to connect, and confirmation notes.
 *
 * On macOS the window buttons sit at the top of the sidebar, beside the title
 * bar. On Linux the system draws its own frame.
 */
import { useEffect } from 'react';
import { Outlet, useNavigate } from 'react-router-dom';
import { getBridge } from '@ui/api/bridge';
import { EngineStatus } from '@ui/components/EngineStatus';
import { RefreshBar } from '@ui/components/RefreshBar';
import { Toasts } from '@ui/components/Toasts';
import { useEngineEvents } from '@ui/api/engine';
import { CommandPalette } from '@ui/palette/CommandPalette';
import { usePalette } from '@ui/palette/usePalette';
import { useShortcuts } from '@ui/palette/useShortcuts';
import { useAppCommand } from '@ui/palette/useAppCommand';
import { APP_COMMANDS, COMMAND_PREFIX } from '@shared/shortcuts';
import { OverlayDialogs } from './OverlayDialogs';
import { RequestCards } from './RequestCards';
import { useOverlays } from './useOverlays';
import { useTheme } from './useTheme';
import { useWindowTitle } from './useWindowTitle';
import styles from './AppShell.module.css';

export function AppShell() {
  useEngineEvents();
  useShortcuts();
  useTheme();
  const palette = usePalette();
  useNavigateRequests();
  useWindowTitle();

  return (
    <div className={styles.shell}>
      <RefreshBar />
      <EngineStatus />
      <div className={styles.body}>
        <Outlet />
      </div>
      {palette.isOpen && <CommandPalette onClose={palette.close} />}
      <OverlayDialogs />
      <RequestCards />
      <Toasts />
    </div>
  );
}

/**
 * The menu bar panel and notifications ask the main window for a screen (a
 * route, or `settings`); the app menu asks it to run a command.
 */
function useNavigateRequests(): void {
  const navigate = useNavigate();
  const { openSettings } = useOverlays();
  const run = useAppCommand();

  useEffect(
    () =>
      getBridge().onNavigate((target) => {
        // The app menu sends commands, such as `cmd:refresh`.
        const command = target.startsWith(COMMAND_PREFIX)
          ? APP_COMMANDS.find((name) => `${COMMAND_PREFIX}${name}` === target)
          : undefined;
        if (command) run(command);
        else if (target === 'settings') openSettings();
        else if (!target.startsWith(COMMAND_PREFIX)) void navigate(target);
      }),
    [navigate, openSettings, run],
  );
}
