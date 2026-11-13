/**
 * Building blocks every project tab shares: the padded column, the intro line
 * with controls on the right, and a highlighted note.
 */
import { createContext, useContext, type ReactNode } from 'react';
import { cx } from '@ui/lib/cx';
import styles from './ProjectTab.module.css';

export { styles as tabStyles };

/** True inside a resource's page, which already pads its content. */
export const InResourcePage = createContext(false);

export function ProjectTab({ children }: { children: ReactNode }) {
  const embedded = useContext(InResourcePage);
  return <div className={cx(styles.tab, embedded && styles.embedded)}>{children}</div>;
}
