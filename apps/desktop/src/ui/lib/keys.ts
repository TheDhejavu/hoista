/**
 * How a shortcut is written for this computer: ⌘K on macOS, Ctrl+K on
 * Linux and Windows.
 */
import { getBridge } from '@ui/api/bridge';

const MAC = getBridge().platform === 'darwin';

/** The key pressed with ⌘ (Ctrl), as the person sees it on the keyboard. */
export function shortcutLabel(key: string): string {
  return `${MAC ? '⌘' : 'Ctrl+'}${key.toUpperCase()}`;
}
