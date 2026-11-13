/**
 * Rows of settings: a titled group of rows, each with a label, an optional
 * hint and a control on the right. Used by project-wide and per-project settings.
 */
import type { ReactNode } from 'react';
import { cx } from '@ui/lib/cx';
import { PillSelect, type Option } from './Select';
import { Skeleton } from './Skeleton';
import styles from './SettingRow.module.css';

/**
 * A card of settings: a bold title and a line about it, the rows, and a grey
 * strip at the bottom for caveats or the card's one action.
 */
export function SettingGroup({
  title,
  description,
  icon,
  note,
  danger = false,
  children,
}: {
  title?: string;
  description?: ReactNode;
  /** A small tile before the title, naming what the card is about. */
  icon?: ReactNode;
  /** The grey strip at the bottom, for caveats or an action. */
  note?: ReactNode;
  /** Red edges, for things like Stop everything. */
  danger?: boolean;
  children?: ReactNode;
}) {
  return (
    <section className={cx(styles.card, danger && styles.danger)}>
      {(title ?? description) && (
        <div className={cx(styles.head, icon !== undefined && styles.withIcon)}>
          {icon !== undefined && (
            <span className={styles.groupIcon} aria-hidden="true">
              {icon}
            </span>
          )}
          <div className={styles.headText}>
            {title && <h3 className={styles.title}>{title}</h3>}
            {description && <p className={styles.description}>{description}</p>}
          </div>
        </div>
      )}
      {children}
      {note && <div className={styles.foot}>{note}</div>}
    </section>
  );
}

/**
 * Settings cards as they will be, while their values load: a title and
 * `rows` rows each, one card per `cards`.
 */
export function SettingGroupSkeleton({ cards = 1, rows = 2 }: { cards?: number; rows?: number }) {
  return (
    <div className={styles.skeletons} role="status" aria-label="Loading">
      {Array.from({ length: cards }, (_, card) => (
        <section key={card} className={styles.card}>
          <div className={styles.head}>
            <div className={styles.headText}>
              <Skeleton width={120} height={14} />
              <Skeleton width={260} height={10} />
            </div>
          </div>
          {Array.from({ length: rows }, (_, row) => (
            <div key={row} className={styles.row}>
              <div className={styles.text}>
                <Skeleton width={140} height={12} />
              </div>
              <Skeleton width={120} height={28} />
            </div>
          ))}
        </section>
      ))}
    </div>
  );
}

export function SettingRow({
  label,
  hint,
  labelFor,
  icon,
  children,
}: {
  label: string;
  hint?: ReactNode;
  /** A logo tile before the label, such as the provider's. */
  icon?: ReactNode;
  /** The id of the control the label names, when there is one. */
  labelFor?: string;
  /** The control: a dropdown, a button, or a fixed value. */
  children?: ReactNode;
}) {
  return (
    <div className={styles.row}>
      {icon && (
        <span className={styles.icon} aria-hidden="true">
          {icon}
        </span>
      )}
      <div className={styles.text}>
        {labelFor ? <label htmlFor={labelFor}>{label}</label> : <span>{label}</span>}
        {hint && <span className={styles.hint}>{hint}</span>}
      </div>
      {children}
    </div>
  );
}

/** A setting chosen from a dropdown, with the row's label naming it. */
export function SelectRow<T>({
  label,
  hint,
  value,
  options,
  onChange,
  isEqual,
}: {
  label: string;
  hint?: ReactNode;
  value: T;
  options: readonly Option<T>[];
  onChange: (value: T) => void;
  isEqual?: (a: T, b: T) => boolean;
}) {
  return (
    <SettingRow label={label} hint={hint}>
      <PillSelect
        label={label}
        hideLabel
        value={value}
        options={options}
        onChange={onChange}
        isEqual={isEqual}
      />
    </SettingRow>
  );
}

/** A value the user can't change, such as "Always asks". */
export function FixedValue({ children }: { children: ReactNode }) {
  return <span className={styles.fixed}>{children}</span>;
}
