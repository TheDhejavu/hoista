/**
 * A page with nothing on it yet, centered in the space: a large quiet icon,
 * what's missing, and the commands that fill it, set inline as `code`.
 */
import type { ReactNode } from 'react';
import styles from './EmptyState.module.css';

export function EmptyState({
  icon,
  title,
  children,
}: {
  /** A Lucide icon; it's drawn large and quiet. */
  icon: ReactNode;
  title: string;
  /** One or two sentences; `code` inside reads as a command or prompt. */
  children: ReactNode;
}) {
  return (
    <section className={styles.empty} aria-label={title}>
      <span className={styles.icon} aria-hidden="true">
        {icon}
      </span>
      <h2 className={styles.title}>{title}</h2>
      <p className={styles.text}>{children}</p>
    </section>
  );
}
