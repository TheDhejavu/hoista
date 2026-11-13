/**
 * A numbered list of steps, such as "make an account, create a token, paste
 * it below". Each step has a title and optional detail under it.
 */
import type { ReactNode } from 'react';
import styles from './Steps.module.css';

export function Steps({ children }: { children: ReactNode }) {
  return <ol className={styles.steps}>{children}</ol>;
}

export function Step({
  number,
  title,
  children,
}: {
  number: number;
  title: ReactNode;
  children?: ReactNode;
}) {
  return (
    <li className={styles.step}>
      <span className={styles.number} aria-hidden="true">
        {number}
      </span>
      <span className={styles.text}>
        <span className={styles.title}>{title}</span>
        {children}
      </span>
    </li>
  );
}
