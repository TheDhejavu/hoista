import { type ReactNode } from 'react';
import { useCopy } from '@ui/lib/useCopy';
import styles from './ResourceScreen.module.css';

/** A value to copy, such as a private domain, with a quiet Copy button. */
export function CopyText({ text, note }: { text: string; note?: string }) {
  const { copied, copy } = useCopy();
  return (
    <span className={styles.copyText}>
      <span className="mono">{text}</span>
      {note && <span className={styles.muted}>{note}</span>}
      <button
        type="button"
        className={styles.copyButton}
        onClick={() => {
          copy(text);
        }}
      >
        {copied ? 'Copied' : 'Copy'}
      </button>
    </span>
  );
}

export function Section({
  title,
  action,
  children,
}: {
  title: string;
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className={styles.section}>
      <div className={styles.sectionHead}>
        <h2>{title}</h2>
        {action}
      </div>
      <div className={styles.box}>{children}</div>
    </section>
  );
}

export function Rows({ rows }: { rows: [string, ReactNode][] }) {
  return (
    <dl className={styles.rows}>
      {rows.map(([term, value]) => (
        <div key={term} className={styles.rowItem}>
          <dt>{term}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}
