/**
 * What the engine found when it checked a new token: a dot for each thing
 * that works, and a plain note for anything missing.
 */
import type { TokenCheck } from '@shared/api';
import { Dot } from './Dot';
import styles from './TokenChecks.module.css';

export function TokenChecks({ checks }: { checks: TokenCheck[] }) {
  return (
    <ul className={styles.checks} aria-label="Token checks">
      {checks.map((check) => (
        <li key={check.label} className={styles.check}>
          <Dot tone={check.ok ? 'good' : 'warn'} />
          <span className={styles.text}>
            <span className={styles.label}>{check.label}</span>
            {check.note && <span>{check.note}</span>}
          </span>
        </li>
      ))}
    </ul>
  );
}
