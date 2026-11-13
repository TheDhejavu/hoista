/**
 * Whether a modal dialog is open anywhere in the window. While one is, the
 * rest of the page can't be clicked: a click there lands on the dialog's
 * backdrop and closes it, so floating cards step aside until it closes.
 */
import { useSyncExternalStore } from 'react';

function anyModal(): boolean {
  return document.querySelector('dialog:modal') !== null;
}

function subscribe(listener: () => void): () => void {
  const observer = new MutationObserver(listener);
  observer.observe(document.body, { subtree: true, childList: true, attributeFilter: ['open'] });
  return () => {
    observer.disconnect();
  };
}

export function useModalOpen(): boolean {
  return useSyncExternalStore(subscribe, anyModal, () => false);
}
