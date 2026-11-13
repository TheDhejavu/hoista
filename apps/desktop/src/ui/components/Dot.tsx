/** A small coloured status dot. */
import { cx } from '@ui/lib/cx';
import styles from './Dot.module.css';

export type Tone = 'good' | 'bad' | 'warn' | 'neutral' | 'hollow' | 'off';

/** `pulse` adds a slow ring, for things that are live right now. */
export function Dot({
  tone,
  small = false,
  pulse = false,
}: {
  tone: Tone;
  small?: boolean;
  pulse?: boolean;
}) {
  return (
    <span
      className={cx(styles.dot, styles[tone], small && styles.small, pulse && styles.pulse)}
      aria-hidden="true"
    />
  );
}
