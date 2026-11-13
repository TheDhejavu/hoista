/**
 * Page layout: padding, a bold title with an optional line above and below
 * it, and actions on the right.
 */
import type { ReactNode } from 'react';
import styles from './Page.module.css';

export function Page({ children }: { children: ReactNode }) {
  return <div className={styles.page}>{children}</div>;
}
