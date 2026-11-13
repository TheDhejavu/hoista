/**
 * Settings and Get help open as dialogs over whatever screen is showing, so
 * the user never loses their place. This holds which one is open, project-wide.
 */
import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from 'react';

/** The sections of the Settings dialog, in the order they are listed. */
export type SettingsSection =
  'general' | 'agents' | 'vault' | 'safety' | 'webhooks' | 'instances' | 'cli' | 'updates';

type Overlay =
  { kind: 'settings'; section: SettingsSection } | { kind: 'help' } | { kind: 'shortcuts' } | null;

interface OverlayState {
  overlay: Overlay;
  /** Opens Settings at a section. */
  openSettings: (section?: SettingsSection) => void;
  openHelp: () => void;
  /** The list of keyboard shortcuts. */
  openShortcuts: () => void;
  close: () => void;
}

const OverlayContext = createContext<OverlayState | null>(null);

export function OverlayProvider({ children }: { children: ReactNode }) {
  const [overlay, setOverlay] = useState<Overlay>(null);

  const openSettings = useCallback<OverlayState['openSettings']>((section = 'general') => {
    setOverlay({ kind: 'settings', section });
  }, []);
  const openHelp = useCallback(() => {
    setOverlay({ kind: 'help' });
  }, []);
  const openShortcuts = useCallback(() => {
    setOverlay({ kind: 'shortcuts' });
  }, []);
  const close = useCallback(() => {
    setOverlay(null);
  }, []);

  const state = useMemo(
    () => ({ overlay, openSettings, openHelp, openShortcuts, close }),
    [overlay, openSettings, openHelp, openShortcuts, close],
  );
  return <OverlayContext value={state}>{children}</OverlayContext>;
}

export function useOverlays(): OverlayState {
  const state = useContext(OverlayContext);
  if (!state) throw new Error('useOverlays must be used inside <OverlayProvider>.');
  return state;
}
