/**
 * The plain-English error from a failed request, in a faint red box with
 * an icon, so it reads as one message wherever it sits rather than loose
 * red text. Renders nothing when there is no error, so it can sit in place
 * unconditionally.
 */
import { TriangleAlert } from 'lucide-react';
import styles from './ErrorMessage.module.css';

export function ErrorMessage({ error }: { error: Error | null | undefined }) {
  if (!error) return null;

  return (
    <div className={styles.error} role="alert">
      <TriangleAlert size={15} aria-hidden="true" className={styles.icon} />
      <span>{error.message}</span>
    </div>
  );
}
