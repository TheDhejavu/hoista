/**
 * Open and close state for the command palette (⌘K), shared project-wide.
 */
import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from 'react';

interface PaletteState {
  isOpen: boolean;
  open: () => void;
  close: () => void;
}

const PaletteContext = createContext<PaletteState | null>(null);

export function PaletteProvider({ children }: { children: ReactNode }) {
  const [isOpen, setOpen] = useState(false);

  const open = useCallback(() => {
    setOpen(true);
  }, []);
  const close = useCallback(() => {
    setOpen(false);
  }, []);
  const state = useMemo(() => ({ isOpen, open, close }), [isOpen, open, close]);

  return <PaletteContext value={state}>{children}</PaletteContext>;
}

export function usePalette(): PaletteState {
  const state = useContext(PaletteContext);
  if (!state) throw new Error('usePalette must be used inside <PaletteProvider>.');
  return state;
}
