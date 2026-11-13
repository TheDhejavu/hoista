/** Pieces both kinds of metrics page share: the ranges and a chart's card. */
import type { ReactNode } from 'react';
import { Skeleton } from '@ui/components/Skeleton';
import type { Option } from '@ui/components/Select';
import styles from './MetricsTab.module.css';
import { HOUR_SECONDS, DAY_SECONDS } from '@shared/time';

export type Range = '1h' | '24h' | '7d' | '30d';

type RangeChoice = Option<Range> & { span: number; step: number };
export const DAY: RangeChoice = {
  value: '24h',
  label: 'Last 24 hours',
  span: DAY_SECONDS,
  step: 900,
};
export const RANGES: RangeChoice[] = [
  { value: '1h', label: 'Last hour', span: HOUR_SECONDS, step: 60 },
  DAY,
  { value: '7d', label: 'Last 7 days', span: 7 * DAY_SECONDS, step: HOUR_SECONDS },
  { value: '30d', label: 'Last 30 days', span: 30 * DAY_SECONDS, step: 21_600 },
];

export function ChartCard({
  label,
  value,
  note,
  side,
  loading = false,
  children,
}: {
  label: string;
  value: string;
  note?: string;
  side?: ReactNode;
  /** Still asking the provider: placeholders where the number and chart go. */
  loading?: boolean;
  children?: ReactNode;
}) {
  return (
    <section className={styles.card} aria-busy={loading || undefined}>
      <div className={styles.cardHead}>
        <div>
          <h3 className={styles.cardLabel}>{label}</h3>
          {loading ? (
            <span className={styles.cardSkeleton} role="status" aria-label="Loading">
              <Skeleton width={90} height={22} />
              <Skeleton width={140} height={10} />
            </span>
          ) : (
            <>
              <div className={styles.cardValue}>{value}</div>
              {note && <div className={styles.cardNote}>{note}</div>}
            </>
          )}
        </div>
        {!loading && side}
      </div>
      {loading ? <Skeleton height={164} className={styles.chartSkeleton} /> : children}
    </section>
  );
}
