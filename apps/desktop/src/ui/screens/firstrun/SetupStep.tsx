/**
 * One setup step beside the rail: "Step 1 of 3", the title and a line
 * under it, the step's content, and a footer with Back on the left and a
 * note and the step's actions on the right.
 */
import type { ReactNode } from 'react';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { cx } from '@ui/lib/cx';
import styles from './Setup.module.css';

export function SetupStep({
  number,
  title,
  lede,
  back,
  note,
  actions,
  error,
  narrow = false,
  children,
}: {
  /** Which of the three rail steps this is, from 1. */
  number: number;
  title: string;
  lede: string;
  /** The Back button, or nothing on a step with nowhere to go back to. */
  back?: ReactNode;
  /** A short line before the actions, such as what's picked. */
  note?: ReactNode;
  actions: ReactNode;
  error?: Error | null;
  /** Content held to a reading width, for a step of text and fields. */
  narrow?: boolean;
  children: ReactNode;
}) {
  return (
    <main className={styles.step} aria-labelledby="setup-step-title">
      <div className={cx(styles.stepBody, narrow && styles.narrow)}>
        <span className={styles.eyebrow}>Step {number} of 3</span>
        <h1 id="setup-step-title" className={styles.title}>
          {title}
        </h1>
        <p className={styles.lede}>{lede}</p>
        {children}
        <ErrorMessage error={error ?? null} />
      </div>
      <footer className={styles.stepFooter}>
        <span>{back}</span>
        <span className={styles.stepActions}>
          {note && <span className={styles.stepNote}>{note}</span>}
          {actions}
        </span>
      </footer>
    </main>
  );
}
