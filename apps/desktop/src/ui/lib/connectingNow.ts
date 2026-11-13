/**
 * The agent the Connect dialog is waiting on right now, if any. Its request
 * is answered inside that dialog, so the cards on other screens leave it out.
 */
import { useSyncExternalStore } from 'react';

let current: string | null = null;
const listeners = new Set<() => void>();

export function setConnectingNow(name: string | null): void {
  current = name;
  for (const listener of listeners) listener();
}

export function useConnectingNow(): string | null {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    () => current,
  );
}
