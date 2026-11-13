/**
 * Shown in place of a screen that failed to render, inside the normal
 * window, so the sidebar keeps working and one broken screen never blanks
 * the app. Gives the message, a way back and a retry.
 */
import { Link, useRouteError } from 'react-router-dom';
import { Button } from '@ui/components/Button';
import styles from './ScreenError.module.css';
import { forgetCache } from '@ui/lib/persistCache';

export function ScreenError() {
  const error = useRouteError();
  const message = error instanceof Error ? error.message : 'Something went wrong.';
  return (
    <div className={styles.wrap} role="alert">
      <h1 className={styles.title}>This screen hit a problem</h1>
      <p className={styles.body}>
        The rest of Hoista still works. Your projects and accounts are unaffected.
      </p>
      <code className={styles.detail}>{message}</code>
      <div className={styles.actions}>
        <Button
          variant="primary"
          onClick={() => {
            // Start from fresh data, in case what was saved is what broke.
            forgetCache();
            window.location.reload();
          }}
        >
          Try again
        </Button>
        <Link to="/" className={styles.link}>
          Go to Projects
        </Link>
      </div>
    </div>
  );
}
