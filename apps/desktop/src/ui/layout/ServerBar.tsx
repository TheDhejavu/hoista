/**
 * A strip above every screen while the app shows one of your remotes, so
 * it's never mistaken for this computer's Hoista, with the way back.
 *
 * If the remote isn't answering (it's asleep, or its provider is down), the
 * strip says so straight away instead of leaving screens waiting: what's on
 * screen is from the last time it answered, and This Mac is one click away.
 */
import { Server, TriangleAlert } from 'lucide-react';
import { useEngineHealth, useRemotes, useShowRemote, useShownRemote } from '@ui/api/settings';
import { Button } from '@ui/components/Button';
import { cx } from '@ui/lib/cx';
import styles from './ServerBar.module.css';

export function ServerBar() {
  const remotes = useRemotes();
  const health = useEngineHealth();
  const show = useShowRemote();
  const shown = useShownRemote();
  if (!shown) return null;
  const { remote, down } = shown;

  return (
    <div className={cx(styles.bar, down && styles.down)} role="status">
      {down ? (
        <TriangleAlert size={14} aria-hidden="true" className={styles.alert} />
      ) : (
        <Server size={14} aria-hidden="true" />
      )}
      <span className={styles.text}>
        {down ? (
          <>
            <strong>{remote.name}</strong> isn’t answering. What you see is from the last time it
            did.
          </>
        ) : (
          <>
            Remote instance <strong>{remote.name}</strong>{' '}
            <span className={styles.provider}>{remote.url.replace(/^https?:\/\//, '')}</span>
          </>
        )}
      </span>
      {down && (
        <Button
          variant="ghost"
          busy={remotes.isFetching || health.isFetching}
          busyLabel="Checking"
          onClick={() => {
            void remotes.refetch();
            void health.refetch();
          }}
        >
          Try again
        </Button>
      )}
      <Button
        variant={down ? 'primary' : 'ghost'}
        busy={show.isPending}
        busyLabel="Switching"
        onClick={() => {
          show.mutate('this_computer');
        }}
      >
        Switch to this Mac
      </Button>
    </div>
  );
}
