/**
 * Applies the Theme setting. "System" leaves the choice to whatever already
 * decides it (the operating system, through `prefers-color-scheme`); Light or
 * Dark pins it until the setting changes back.
 */
import { useEffect } from 'react';
import { useSettings } from '@ui/api/settings';

export function useTheme(): void {
  const setting = useSettings().data?.general.theme ?? 'system';
  const theme = setting;

  useEffect(() => {
    if (theme === 'system') return;

    const root = document.documentElement;
    root.dataset.theme = theme;
    return () => {
      delete root.dataset.theme;
    };
  }, [theme]);
}
