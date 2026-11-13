/**
 * Says plainly when Hoista's engine isn't answering, instead of showing
 * empty screens as if you had no projects.
 */
import { useEngineHealth, useShownRemote } from '@ui/api/settings';
import { Button } from './Button';
import { Mascot } from './Mascot';
import styles from './EngineStatus.module.css';

export function EngineStatus() {
  const health = useEngineHealth();
  // While a remote is shown, the strip above the screen says it isn't
  // answering and offers the way back: this banner is about this Mac only.
  const shown = useShownRemote();
  if (shown || !health.isError) return null;
  return (
    <div className={styles.banner} role="alert">
      <Mascot mood="asleep" size={36} />
      <div className={styles.text}>
        <strong>Hoista’s engine isn’t running.</strong> Your projects keep running at their
        providers; Hoista and your agents can’t reach them until it’s back. Start it with{' '}
        <code>hoist daemon restart</code>, or quit and reopen Hoista.
      </div>
      <Button busy={health.isFetching} busyLabel="Checking" onClick={() => void health.refetch()}>
        Check again
      </Button>
    </div>
  );
}
