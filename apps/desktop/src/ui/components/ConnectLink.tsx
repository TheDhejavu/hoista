/**
 * The provider and Hoista side by side, joined by a link: the line runs while
 * Hoista checks the access, and turns into a green check once it's
 * connected. Drawn on the Connect dialog and its done state.
 */
import { Check, Link2 } from 'lucide-react';
import type { ServiceId } from '@shared/api';
import { cx } from '@ui/lib/cx';
import { LogoMark } from './icons';
import { ServiceIcon } from './ServiceIcon';
import styles from './ConnectLink.module.css';

type ConnectState = 'idle' | 'checking' | 'connected';

export function ConnectLink({
  service,
  name,
  state,
}: {
  service: ServiceId;
  name: string;
  state: ConnectState;
}) {
  const label =
    state === 'connected'
      ? `${name} is connected to Hoista`
      : state === 'checking'
        ? `Connecting ${name} to Hoista`
        : `${name} and Hoista`;

  return (
    <div className={cx(styles.link, styles[state])} role="img" aria-label={label}>
      <span className={styles.tile}>
        <ServiceIcon id={service} size={30} />
      </span>
      <span className={styles.line} />
      <span className={styles.knot}>
        <Link2 size={16} aria-hidden="true" />
        {state === 'connected' && (
          <span className={styles.check}>
            <Check size={10} strokeWidth={3.5} aria-hidden="true" />
          </span>
        )}
      </span>
      <span className={styles.line} />
      <span className={cx(styles.tile, styles.hoista)}>
        <LogoMark size={26} />
      </span>
    </div>
  );
}
