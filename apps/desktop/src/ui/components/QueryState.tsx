/**
 * Renders a query's loading and error states, and its data once it arrives.
 * While it loads: nothing for the first 300ms, then `skeleton`, a placeholder
 * shaped like what's coming (a few lines unless the screen gives its own).
 * An error says what went wrong and offers to try again.
 */
import type { ReactNode } from 'react';
import type { UseQueryResult } from '@tanstack/react-query';
import { useDelayedFlag } from '@ui/lib/useDelayedFlag';
import { Button } from './Button';
import { SkeletonLines } from './Skeleton';
import styles from './QueryState.module.css';

export function QueryState<T>({
  query,
  skeleton,
  children,
}: {
  query: UseQueryResult<T>;
  /** Shaped like the content, so nothing moves when it arrives. */
  skeleton?: ReactNode;
  children: (data: T) => ReactNode;
}) {
  // The placeholder fades in after 300ms (in CSS), so a quick answer never
  // shows it; once it has shown, it stays a moment rather than blinking off.
  const shown = useDelayedFlag(query.isPending);
  if (query.isPending || (shown && query.isSuccess)) {
    return (
      <div className={styles.loading} role="status" aria-label="Loading">
        {skeleton ?? <SkeletonLines />}
      </div>
    );
  }
  // A refresh that failed keeps what was on screen (such as data saved from
  // last time, while a remote isn't answering); only no data at all is an error.
  const data = query.data;
  if (data === undefined) {
    if (!query.isError) return null;
    return (
      <div className={styles.error} role="alert">
        <p>{query.error.message}</p>
        <Button
          disabled={query.isFetching}
          onClick={() => {
            void query.refetch();
          }}
        >
          {query.isFetching ? 'Trying' : 'Try again'}
        </Button>
      </div>
    );
  }
  return <>{children(data)}</>;
}
