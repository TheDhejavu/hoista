/**
 * Buttons and button-styled links.
 *
 * Use `accent` for the one main action on a screen, `primary` for other
 * strong actions, `secondary` for the rest, `ghost` for quiet actions such
 * as Cancel, and `danger` only to confirm something destructive.
 */
import type { ButtonHTMLAttributes, ReactNode } from 'react';
import { Link } from 'react-router-dom';
import { cx } from '@ui/lib/cx';
import { useDelayedFlag } from '@ui/lib/useDelayedFlag';
import styles from './Button.module.css';

/** `quietDanger` is a red text button, for a destructive action that isn't the main one. */
type Variant = 'accent' | 'primary' | 'secondary' | 'ghost' | 'danger' | 'quietDanger';

interface StyleProps {
  variant?: Variant;
  large?: boolean;
  /**
   * Bigger still, for full-window steps such as first-run setup: `xl` for a
   * step's own actions, `hero` for the one action on a welcome screen.
   */
  size?: 'xl' | 'hero';
}

function classNames({ variant = 'secondary', large = false, size }: StyleProps): string {
  return cx(styles.button, styles[variant], large && styles.large, size && styles[size]);
}

/**
 * `busy` shows the action is under way: a spinner, the button disabled, and
 * `busyLabel` (such as "Redeploying") in place of the label when given.
 * Quick work doesn't flash any of that: the button only stops taking clicks
 * at first, and the spinner shows once the work has taken 300ms (then stays
 * at least 400ms).
 */
export function Button({
  variant,
  large,
  size,
  type = 'button',
  busy = false,
  busyLabel,
  disabled,
  children,
  ...rest
}: StyleProps &
  ButtonHTMLAttributes<HTMLButtonElement> & { busy?: boolean; busyLabel?: ReactNode }) {
  const showBusy = useDelayedFlag(busy);
  const blocked = busy || showBusy;
  return (
    <button
      type={type}
      className={classNames({ variant, large, size })}
      disabled={disabled || showBusy}
      aria-disabled={blocked || undefined}
      aria-busy={blocked || undefined}
      {...rest}
      onClick={
        blocked
          ? (event) => {
              event.preventDefault();
            }
          : rest.onClick
      }
    >
      {showBusy && <span className={styles.spinner} aria-hidden="true" />}
      {showBusy && busyLabel ? busyLabel : children}
    </button>
  );
}

/** A link inside the app that looks like a button. */
export function ButtonLink({
  to,
  variant,
  large,
  children,
}: StyleProps & { to: string; children: ReactNode }) {
  return (
    <Link to={to} className={classNames({ variant, large })}>
      {children}
    </Link>
  );
}
