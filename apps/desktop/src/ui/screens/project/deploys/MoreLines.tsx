/**
 * A quiet tag after a commit's subject saying the message goes on, so a
 * one-line row never hides that it was cut. The rest shows when the deploy
 * is opened.
 */
import { extraLines } from '@ui/lib/commitMessage';
import styles from './MoreLines.module.css';

export function MoreLines({ message }: { message: string | null | undefined }) {
  const lines = extraLines(message);
  if (lines === 0) return null;
  return (
    <span className={styles.more} title="Open the deploy to read the whole message">
      +{lines} {lines === 1 ? 'line' : 'lines'}
    </span>
  );
}
