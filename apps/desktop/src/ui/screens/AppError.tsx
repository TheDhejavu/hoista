/**
 * Shown in place of the whole window when the project's frame itself fails
 * (not one screen: that's ScreenError). Says what happened and reloads.
 */
import { useRouteError } from 'react-router-dom';
import { Button } from '@ui/components/Button';
import styles from './ScreenError.module.css';
import { forgetCache } from '@ui/lib/persistCache';

export function AppError() {
  const error = useRouteError();
  const message = error instanceof Error ? error.message : 'Something went wrong.';
  return (
    <div className={styles.whole}>
      <div className={styles.wrap} role="alert">
        <h1 className={styles.title}>Hoista hit a problem</h1>
        <p className={styles.body}>
          Your projects and accounts are unaffected. Reloading the window usually fixes it.
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
            Reload Hoista
          </Button>
        </div>
      </div>
    </div>
  );
}
