/** A raised box. `tone` tints the border to draw the eye. */
import type { ReactNode } from 'react';
import { cx } from '@ui/lib/cx';
import styles from './Panel.module.css';

export function Panel({
  children,
  padded = false,
  tone,
  className = '',
}: {
  children: ReactNode;
  padded?: boolean;
  tone?: 'good' | 'warn' | 'bad';
  className?: string;
}) {
  return (
    <div className={cx(styles.panel, padded && styles.padded, tone && styles[tone], className)}>
      {children}
    </div>
  );
}
