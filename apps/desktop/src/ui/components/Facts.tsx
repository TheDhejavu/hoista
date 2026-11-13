/** A labelled list of facts: a term on the left, its value on the right. */
import type { ReactNode } from 'react';
import { cx } from '@ui/lib/cx';
import styles from './Facts.module.css';

export function Facts({
  compact = false,
  children,
}: {
  /** Term and value on one line, value on the right, for narrow side panels. */
  compact?: boolean;
  children: ReactNode;
}) {
  return <dl className={cx(styles.facts, compact && styles.compact)}>{children}</dl>;
}

export function Fact({
  term,
  mono = false,
  children,
}: {
  term: string;
  /** Show the term in the mono font, for commands and ids. */
  mono?: boolean;
  children: ReactNode;
}) {
  return (
    <div className={styles.fact}>
      <dt className={cx(styles.term, mono && styles.mono)}>{term}</dt>
      <dd className={styles.value}>{children}</dd>
    </div>
  );
}
