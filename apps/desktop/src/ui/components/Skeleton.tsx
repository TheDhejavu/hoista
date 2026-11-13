/**
 * Placeholders shaped like what is on its way: a number, a few lines, a
 * chart or table rows. The same shimmer as a loading page, so the screen
 * keeps its layout while a provider answers and nothing jumps when it does.
 */
import type { CSSProperties } from 'react';
import { cx } from '@ui/lib/cx';
import styles from './Skeleton.module.css';

/**
 * One block, such as a number (`width` 72, `height` 22) or a chart.
 * `later`: it fades in only after 300ms, so a quick answer never flashes it.
 */
export function Skeleton({
  width,
  height = 12,
  later = false,
  className,
}: {
  width?: number | string;
  height?: number | string;
  later?: boolean;
  className?: string;
}) {
  const style: CSSProperties = { width, height };
  return (
    <span
      className={cx(styles.block, later && styles.later, className)}
      style={style}
      aria-hidden="true"
    />
  );
}

/** A few lines of text, each shorter than the last. */
export function SkeletonLines({ count = 3 }: { count?: number }) {
  return (
    <span className={styles.lines} role="status" aria-label="Loading">
      {Array.from({ length: count }, (_, index) => (
        <span
          key={index}
          className={styles.block}
          style={{ width: `${String(100 - index * (count > 1 ? 45 / (count - 1) : 0))}%` }}
        />
      ))}
    </span>
  );
}

/** Table rows: `count` rows of `columns` cells. */
export function SkeletonRows({ count = 3, columns = 4 }: { count?: number; columns?: number }) {
  return (
    <span className={styles.rows} role="status" aria-label="Loading">
      {Array.from({ length: count }, (_, row) => (
        <span key={row} className={styles.row}>
          {Array.from({ length: columns }, (_, column) => (
            <span
              key={column}
              className={styles.block}
              style={{ width: `${String(column === 0 ? 70 : 50)}%` }}
            />
          ))}
        </span>
      ))}
    </span>
  );
}

/**
 * The body of a real table while its rows load: `count` rows, one
 * placeholder per column, each as wide as `widths` says (`null` for an empty
 * cell, such as the actions column). The table keeps its own header and
 * frame, so only the rows are placeholders.
 */
export function SkeletonTableBody({
  count = 3,
  widths,
  height = 12,
}: {
  count?: number;
  widths: (number | string | null)[];
  height?: number;
}) {
  return (
    <tbody className={styles.tableBody} role="status" aria-label="Loading">
      {Array.from({ length: count }, (_, row) => (
        <tr key={row}>
          {widths.map((width, column) => (
            <td key={column}>{width !== null && <Skeleton width={width} height={height} />}</td>
          ))}
        </tr>
      ))}
    </tbody>
  );
}
