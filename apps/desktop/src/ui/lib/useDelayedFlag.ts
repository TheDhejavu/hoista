/**
 * Whether to show a spinner, so quick work never flashes one: it shows only
 * once `on` has lasted `delay` ms, and once shown it stays at least `min` ms
 * so it never blinks off as soon as it appears.
 */
import { useEffect, useRef, useState } from 'react';

const SPINNER_DELAY_MS = 300;
const SPINNER_MIN_MS = 400;

export function useDelayedFlag(
  on: boolean,
  { delay = SPINNER_DELAY_MS, min = SPINNER_MIN_MS }: { delay?: number; min?: number } = {},
): boolean {
  const [shown, setShown] = useState(false);
  const shownAt = useRef(0);

  useEffect(() => {
    if (on && !shown) {
      const timer = setTimeout(() => {
        shownAt.current = Date.now();
        setShown(true);
      }, delay);
      return () => {
        clearTimeout(timer);
      };
    }
    if (!on && shown) {
      const left = Math.max(0, min - (Date.now() - shownAt.current));
      const timer = setTimeout(() => {
        setShown(false);
      }, left);
      return () => {
        clearTimeout(timer);
      };
    }
    return undefined;
  }, [on, shown, delay, min]);

  return shown;
}
