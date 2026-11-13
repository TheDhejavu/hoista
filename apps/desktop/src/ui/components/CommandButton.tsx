/**
 * Copies the `hoist` command for what's on screen. Shows the command on hover
 * and "Copied" once it's on the clipboard.
 */
import { Check, SquareTerminal } from 'lucide-react';
import { useCopy } from '@ui/lib/useCopy';
import styles from './CommandButton.module.css';

export function CommandButton({ command }: { command: string }) {
  const { copied, copy } = useCopy();
  return (
    <button
      type="button"
      className={styles.button}
      title={command}
      aria-label={`Copy the command: ${command}`}
      onClick={() => {
        copy(command);
      }}
    >
      {copied ? (
        <Check size={14} aria-hidden="true" />
      ) : (
        <SquareTerminal size={14} aria-hidden="true" />
      )}
      <span>{copied ? 'Copied' : 'Copy command'}</span>
    </button>
  );
}
