/**
 * A thin bar across the top of the window while something already on screen
 * is being read again, so the page stays put instead of blanking. Polls that
 * run on a timer don't show it, and nothing shows for reads under 300ms.
 */
import { useIsFetching, type Query } from '@tanstack/react-query';
import { useDelayedFlag } from '@ui/lib/useDelayedFlag';
import styles from './RefreshBar.module.css';

/** A read of something already shown, that isn't a poll on a timer. */
function isRefresh(query: Query): boolean {
  return (
    query.state.data !== undefined &&
    !query.observers.some((observer) => Boolean(observer.options.refetchInterval))
  );
}

export function RefreshBar() {
  const refreshing = useIsFetching({ predicate: isRefresh }) > 0;
  const shown = useDelayedFlag(refreshing);
  if (!shown) return null;
  return (
    <div className={styles.bar} role="progressbar" aria-label="Refreshing">
      <span />
    </div>
  );
}
