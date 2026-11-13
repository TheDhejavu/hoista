/**
 * Runs one of the app's commands (see `@shared/shortcuts`), from a key
 * press or from the app menu.
 */
import { useCallback } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { useNavigate } from 'react-router-dom';
import { COMMAND_ROUTES, type AppCommand } from '@shared/shortcuts';
import { useOverlays } from '@ui/layout/useOverlays';
import { showToast } from '@ui/lib/toast';
import { usePalette } from './usePalette';

export function useAppCommand(): (command: AppCommand) => void {
  const { open } = usePalette();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { openSettings, openHelp, openShortcuts } = useOverlays();

  return useCallback(
    (command: AppCommand) => {
      const route = COMMAND_ROUTES[command];
      if (route) {
        void navigate(route);
        return;
      }
      switch (command) {
        case 'palette':
          open();
          return;
        case 'settings':
          openSettings();
          return;
        case 'help':
          openHelp();
          return;
        case 'shortcuts':
          openShortcuts();
          return;
        case 'back':
          void navigate(-1);
          return;
        case 'forward':
          void navigate(1);
          return;
        case 'refresh':
          // Every list and page asks the engine again.
          void queryClient.invalidateQueries().then(() => {
            showToast('Refreshed');
          });
          return;
        default:
          return;
      }
    },
    [open, navigate, queryClient, openSettings, openHelp, openShortcuts],
  );
}
