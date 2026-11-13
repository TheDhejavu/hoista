/**
 * A provider operation's error. A provider's command-line tool that's still being
 * set up for first use isn't an error: it says so plainly, with Try again.
 * A provider that can't read a private repository says what to approve, opens
 * the page for it, and tries again on its own while you approve.
 */
import { useEffect, useRef, useState } from 'react';
import { openExternal } from '@ui/api/system';
import { Button } from '@ui/components/Button';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { EngineError } from '@shared/engine';
import styles from './ProviderError.module.css';

export function ProviderError({
  error,
  onRetry,
  retrying = false,
}: {
  error: Error | null | undefined;
  onRetry?: () => void;
  retrying?: boolean;
}) {
  if (error instanceof EngineError && error.code === 'provider.tool_installing') {
    return (
      <div className={styles.settingUp} role="status">
        <p>{error.message}</p>
        {onRetry && (
          <Button busy={retrying} busyLabel="Checking" onClick={onRetry}>
            Try again
          </Button>
        )}
      </div>
    );
  }
  if (error instanceof EngineError && error.code === 'provider.git_access_needed') {
    return <GitAccess message={error.message} onRetry={onRetry} retrying={retrying} />;
  }
  return <ErrorMessage error={error} />;
}

/** How long Hoista keeps trying again after the approval page opens. */
const WAIT_MS = 5 * 60 * 1000;
const EVERY_MS = 10 * 1000;

function GitAccess({
  message,
  onRetry,
  retrying,
}: {
  message: string;
  onRetry?: () => void;
  retrying: boolean;
}) {
  // The page to approve on comes in the engine's own sentence.
  const page = /https:\/\/\S+/.exec(message)?.[0]?.replace(/[.)]+$/, '') ?? null;
  const text = page
    ? message
        .replace(page, '')
        .replace(/\s+([.,])/g, '$1')
        .trim()
    : message;
  const [since, setSince] = useState<number | null>(null);
  const waiting = since !== null;
  // The latest retry, so the timer isn't restarted on every render.
  const retry = useRef(onRetry);
  useEffect(() => {
    retry.current = onRetry;
  }, [onRetry]);

  useEffect(() => {
    if (since === null) return;
    const timer = window.setInterval(() => {
      if (Date.now() - since >= WAIT_MS) {
        window.clearInterval(timer);
        setSince(null);
        return;
      }
      retry.current?.();
    }, EVERY_MS);
    return () => {
      window.clearInterval(timer);
    };
  }, [since]);

  return (
    <div className={styles.access} role="status">
      <p>{text}</p>
      <div className={styles.accessActions}>
        {page && (
          <Button
            variant="accent"
            onClick={() => {
              void openExternal(page);
              setSince(Date.now());
            }}
          >
            Give access on GitHub
          </Button>
        )}
        {onRetry && (
          <Button busy={retrying} busyLabel="Checking" onClick={onRetry}>
            Try again
          </Button>
        )}
      </div>
      {waiting && (
        <p className={styles.waiting}>
          Waiting for you to approve. Hoista tries again every few seconds.
        </p>
      )}
    </div>
  );
}
