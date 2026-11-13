/** The title and intro paragraph at the top of each Settings section. */
import type { ReactNode } from 'react';
import styles from './SectionHeader.module.css';

/**
 * The top of one settings section. The window's header already shows the
 * title, so this only holds an action, when there is one.
 */
export function SectionHeader({ title, action }: { title: string; action?: ReactNode }) {
  if (!action) return null;
  return (
    <div className={styles.header} aria-label={title}>
      {action}
    </div>
  );
}
