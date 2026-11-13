/**
 * The Projects table's optional columns and the Columns dropdown that turns them
 * on and off. The Project column is always shown.
 */
import { Check, ChevronDown } from 'lucide-react';
import { usePopover } from '@ui/lib/usePopover';
import styles from './columns.module.css';

/** Columns you can turn on and off, in the table's order. */
export const COLUMNS = [
  { id: 'status', label: 'Status' },
  { id: 'providers', label: 'Providers' },
  { id: 'mode', label: 'Managed by' },
  { id: 'runs_on', label: 'Runs on' },
  { id: 'requests', label: 'Requests · 7 days' },
  { id: 'last_deploy', label: 'Last deploy' },
] as const;

export type ColumnId = (typeof COLUMNS)[number]['id'];

/** Project and the row menu are always there, so they count toward the total. */
const FIXED_COLUMNS = 2;

export const DEFAULT_COLUMNS: ColumnId[] = COLUMNS.map((column) => column.id).filter(
  (id) => id !== 'mode' && id !== 'runs_on',
);

const IDS = new Set<unknown>(COLUMNS.map((column) => column.id));

export function isColumnList(value: unknown): value is ColumnId[] {
  return Array.isArray(value) && value.every((id) => IDS.has(id));
}

export function ColumnsMenu({
  value,
  onChange,
}: {
  value: ColumnId[];
  onChange: (columns: ColumnId[]) => void;
}) {
  const { open, setOpen, ref } = usePopover<HTMLDivElement>();
  const shown = new Set(value);

  function toggle(id: ColumnId): void {
    const next = new Set(shown);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    // Keep the table's own order, whatever order they were ticked in.
    onChange(COLUMNS.map((column) => column.id).filter((column) => next.has(column)));
  }

  return (
    <div ref={ref} className={styles.wrap}>
      <button
        type="button"
        className={styles.button}
        aria-haspopup="true"
        aria-expanded={open}
        onClick={() => {
          setOpen(!open);
        }}
      >
        <span className={styles.label}>Columns</span>
        <span>
          {value.length + FIXED_COLUMNS} of {COLUMNS.length + FIXED_COLUMNS}
        </span>
        <ChevronDown size={13} aria-hidden="true" className={styles.chevron} />
      </button>
      {open && (
        <div className={styles.menu} role="group" aria-label="Columns">
          {COLUMNS.map((column) => (
            <label key={column.id} className={styles.option}>
              <input
                type="checkbox"
                checked={shown.has(column.id)}
                onChange={() => {
                  toggle(column.id);
                }}
              />
              <span className={styles.box} aria-hidden="true">
                <Check size={11} strokeWidth={3} />
              </span>
              {column.label}
            </label>
          ))}
        </div>
      )}
    </div>
  );
}
