/**
 * A watched resource's own numbers, as its provider reports them: requests and
 * server errors by status, response-time percentiles, CPU, memory and disk.
 * Shared by the app overview and the resource page. A card whose number the
 * provider doesn't give is left out rather than shown empty.
 */
import { useMemo, useState, type ReactNode } from 'react';
import type { ProviderTarget, Resource } from '@shared/api';
import { useProviderRead } from '@ui/api/providerOperations';
import { Skeleton } from '@ui/components/Skeleton';
import { cx } from '@ui/lib/cx';
import { formatCount, formatGigabytes } from '@ui/lib/format';
import styles from './ProjectOverview.module.css';
import { MINUTE_MS, DAY_MS, HOUR_SECONDS, DAY_SECONDS, floorTo, providerTime } from '@shared/time';

export function Card({
  label,
  value,
  loading = false,
  children,
}: {
  label: string;
  value: string;
  /** Still asking the provider: a quiet placeholder where the number goes. */
  loading?: boolean;
  children?: ReactNode;
}) {
  return (
    <div className={styles.card} aria-busy={loading || undefined}>
      <div className={styles.cardLabel}>{label}</div>
      {loading ? (
        <span className={styles.cardLoading} role="status" aria-label="Loading">
          <Skeleton width={72} height={22} />
        </span>
      ) : (
        <div className={styles.cardValue}>{value}</div>
      )}
      {!loading && children}
    </div>
  );
}

/**
 * A small bar chart, the newest bar in the chart colour (blue). Under it, when the bars start
 * and end; pointing at a bar shows its time and value there instead.
 */
export function Bars({
  values,
  labels,
  format = (value) => formatCount(value),
  from,
  to,
}: {
  values: number[];
  /** Per bar, for example `Mon 2 Nov` or `14:00`. */
  labels?: string[];
  format?: (value: number) => string;
  from?: string;
  to?: string;
}) {
  const [hovered, setHovered] = useState<number | null>(null);
  // Scale to the largest value, not to 1: CPU in vCPUs and memory in GB are
  // small numbers, and against 1 every bar looked the same.
  const max = Math.max(...values, 0) || 1;
  const pointed = hovered === null ? undefined : values[hovered];
  return (
    <div
      onMouseLeave={() => {
        setHovered(null);
      }}
    >
      <div className={styles.bars} aria-hidden="true">
        {values.map((value, index) => (
          <i
            key={index}
            className={cx(
              index === values.length - 1 && styles.barToday,
              hovered === index && styles.barHover,
            )}
            style={{ height: `${(30 + (value / max) * 70).toFixed(0)}%` }}
            onMouseEnter={() => {
              setHovered(index);
            }}
          />
        ))}
      </div>
      {(from !== undefined || to !== undefined || labels) && (
        <div className={styles.barEnds}>
          {pointed !== undefined && hovered !== null ? (
            <span className={styles.barPointed}>
              {labels?.[hovered] ? `${labels[hovered]} · ` : ''}
              <b>{format(pointed)}</b>
            </span>
          ) : (
            <>
              <span>{from}</span>
              <span>{to}</span>
            </>
          )}
        </div>
      )}
    </div>
  );
}

/** Bars in a small chart: the newest samples. */
const BARS = 14;

const DAY = new Intl.DateTimeFormat('en-GB', { weekday: 'short', day: 'numeric', month: 'short' });
const HOUR = new Intl.DateTimeFormat('en-GB', { hour: '2-digit', minute: '2-digit' });

/** A sample time from the provider (seconds or an ISO string) as a date. */
function when(ts: number | string): Date {
  return new Date(providerTime(ts));
}

/** The last `n` days, oldest first, for bars of one value a day ending today. */
export function lastDays(n: number): string[] {
  const now = Date.now();
  return Array.from({ length: n }, (_, index) =>
    DAY.format(new Date(now - (n - 1 - index) * DAY_MS)),
  );
}

interface Sample {
  ts: number | string;
  value: number;
}
type ByStatus = { statusCode: number; samples: Sample[] }[];
type Durations = { samples: { p50: number | null; p95: number | null; p99: number | null }[] };
type Measured = { measurement: string; values: Sample[] }[];

/** The window the overview reads, to the minute so reads are shared. */
function useWindow() {
  const [opened] = useState(() => floorTo(Date.now(), MINUTE_MS));
  return useMemo(() => {
    const iso = (ms: number) => new Date(ms).toISOString();
    return {
      now: iso(opened),
      day: iso(opened - DAY_MS),
      week: iso(opened - 7 * DAY_MS),
    };
  }, [opened]);
}

/** vCPUs at a precision that still shows a nearly idle service. */
function vcpu(value: number): string {
  if (value === 0) return '0';
  if (value < 0.001) return '< 0.001';
  return value < 0.1 ? value.toPrecision(2) : value.toFixed(2);
}

function sum(values: number[]): number {
  return values.reduce((total, value) => total + value, 0);
}

function mean(values: number[]): number | null {
  return values.length ? sum(values) / values.length : null;
}

/**
 * Railway's own numbers for a watched project: requests and server errors by
 * status, response-time percentiles, CPU, memory and disk. A card whose
 * number Railway doesn't give is left out rather than shown empty.
 */
export function ProviderStats({
  code,
  database,
  lastCard,
}: {
  code: Resource;
  database: Resource | null;
  lastCard: ReactNode;
}) {
  const window = useWindow();
  const target: ProviderTarget = { kind: 'resource', id: code.id };
  // A database serves no HTTP, so it has no requests or server errors to count.
  const http = code.kind !== 'database';
  const week = useProviderRead<ByStatus>(http ? target : null, 'httpMetricsGroupedByStatus', {
    startDate: window.week,
    endDate: window.now,
    stepSeconds: DAY_SECONDS,
  });
  const day = useProviderRead<ByStatus>(http ? target : null, 'httpMetricsGroupedByStatus', {
    startDate: window.day,
    endDate: window.now,
    stepSeconds: HOUR_SECONDS,
  });
  const durations = useProviderRead<Durations>(http ? target : null, 'httpDurationMetrics', {
    startDate: window.day,
    endDate: window.now,
    stepSeconds: DAY_SECONDS,
  });
  const used = useProviderRead<Measured>(target, 'metrics', {
    startDate: window.day,
    endDate: window.now,
    sampleRateSeconds: HOUR_SECONDS,
    measurements: ['CPU_USAGE', 'MEMORY_USAGE_GB', 'DISK_USAGE_GB'],
  });
  const disk = useProviderRead<Measured>(
    database ? { kind: 'resource', id: database.id } : null,
    'metrics',
    {
      startDate: window.day,
      endDate: window.now,
      sampleRateSeconds: HOUR_SECONDS,
      measurements: ['DISK_USAGE_GB'],
    },
  );

  const daily = new Map<string, { ts: number | string; value: number }>();
  for (const status of week.data ?? []) {
    for (const sample of status.samples) {
      const key = String(sample.ts);
      const day = daily.get(key);
      daily.set(key, { ts: sample.ts, value: (day?.value ?? 0) + sample.value });
    }
  }
  const dayEntries = [...daily.values()].sort(
    (a, b) => when(a.ts).getTime() - when(b.ts).getTime(),
  );
  const days = dayEntries.map((day) => day.value);
  const dayLabels = dayEntries.map((day) => DAY.format(when(day.ts)));
  const failing = (day.data ?? []).filter((status) => status.statusCode >= 500);
  const errors = sum(failing.flatMap((status) => status.samples.map((sample) => sample.value)));
  const worst = [...failing].sort(
    (a, b) => sum(b.samples.map((x) => x.value)) - sum(a.samples.map((x) => x.value)),
  )[0];
  const latency = durations.data?.samples.at(-1);
  const samplesOf = (data: Measured | undefined, name: string) =>
    (data ?? []).find((item) => item.measurement === name)?.values ?? [];
  const series = (data: Measured | undefined, name: string) =>
    samplesOf(data, name).map((x) => x.value);
  const cpu = series(used.data, 'CPU_USAGE');
  const memory = series(used.data, 'MEMORY_USAGE_GB');
  const hours = (name: string) =>
    samplesOf(used.data, name)
      .slice(-BARS)
      .map((sample) => HOUR.format(when(sample.ts)));
  const mb = (gb: number) => `${String(Math.round(gb * 1000))} MB`;
  const diskUsed = (
    database ? series(disk.data, 'DISK_USAGE_GB') : series(used.data, 'DISK_USAGE_GB')
  ).at(-1);
  // A service without a volume reports 0: nothing worth a card.
  const diskShown = diskUsed !== undefined && diskUsed > 0 ? diskUsed : undefined;
  const ms = (value: number | null | undefined) =>
    value === null || value === undefined ? '—' : `${Math.round(value).toLocaleString('en-US')} ms`;

  return (
    <>
      <div className={styles.stats}>
        {http && week.isPending && <Card label="Requests · 7 days" value="" loading />}
        {week.data && (
          <Card label="Requests · 7 days" value={formatCount(sum(days))}>
            {days.length > 1 && (
              <Bars
                values={days}
                labels={dayLabels}
                format={(value) => `${formatCount(value)} requests`}
                from={dayLabels[0]}
                to="Today"
              />
            )}
          </Card>
        )}
        {http && day.isPending && <Card label="Server errors · 24h" value="" loading />}
        {day.data && (
          <Card label="Server errors · 24h" value={formatCount(errors)}>
            {worst && <div className={cx('mono', styles.note)}>mostly {worst.statusCode}</div>}
          </Card>
        )}
        {http && durations.isPending && <Card label="Response time · 24h" value="" loading />}
        {latency && latency.p50 !== null && (
          <Card label="Response time · 24h" value={ms(latency.p50)}>
            <div className={styles.small}>
              p50 · p95 {ms(latency.p95)} · p99 {ms(latency.p99)}
            </div>
          </Card>
        )}
        {lastCard}
      </div>
      {used.isPending && (
        <div className={cx(styles.stats, styles.statsRow)}>
          <Card label="CPU · 24h" value="" loading />
          <Card label="Memory · 24h" value="" loading />
          <Card label="Disk" value="" loading />
        </div>
      )}
      {(cpu.length > 0 || memory.length > 0 || diskShown !== undefined) && (
        <div className={cx(styles.stats, styles.statsRow)}>
          {cpu.length > 0 && (
            <Card label="CPU · 24h" value={vcpu(mean(cpu) ?? 0)}>
              <div className={styles.small}>vCPU on average</div>
              <Bars
                values={cpu.slice(-BARS)}
                labels={hours('CPU_USAGE')}
                format={(value) => `${vcpu(value)} vCPU`}
                from={hours('CPU_USAGE')[0]}
                to="Now"
              />
            </Card>
          )}
          {memory.length > 0 && (
            <Card label="Memory · 24h" value={`${Math.round((mean(memory) ?? 0) * 1000)} MB`}>
              <div className={styles.small}>on average</div>
              <Bars
                values={memory.slice(-BARS)}
                labels={hours('MEMORY_USAGE_GB')}
                format={mb}
                from={hours('MEMORY_USAGE_GB')[0]}
                to="Now"
              />
            </Card>
          )}
          {diskShown !== undefined && (
            <Card label="Disk" value={formatGigabytes(diskShown)}>
              <div className={styles.small}>used{database ? ` by ${database.name}` : ''}</div>
            </Card>
          )}
        </div>
      )}
    </>
  );
}
