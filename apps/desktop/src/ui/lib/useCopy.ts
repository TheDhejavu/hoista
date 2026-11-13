/**
 * Copies text to the clipboard and says so for a moment, for Copy buttons.
 */
import { useEffect, useRef, useState } from 'react';

/** How long "Copied" shows before the button goes back to "Copy". */
const COPIED_FOR_MS = 1500;

export function useCopy(): { copied: boolean; copy: (text: string) => void } {
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);

  useEffect(
    () => () => {
      clearTimeout(timer.current);
    },
    [],
  );

  function copy(text: string): void {
    void navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      clearTimeout(timer.current);
      timer.current = setTimeout(() => {
        setCopied(false);
      }, COPIED_FOR_MS);
    });
  }

  return { copied, copy };
}
