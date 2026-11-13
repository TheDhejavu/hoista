/**
 * Metrics for a provider the engine reads as charts: one card per chart, its
 * headline worked out the way the chart says (added up, averaged or the
 * newest point), and a line saying what the provider couldn't answer.
 */
import { useState } from 'react';
import type { ProviderChart, MetricUnit } from '@engine-api';
import type { Resource } from '@shared/api';
import { useProviderMetrics } from '@ui/api/metrics';
import { MetricsNotice } from './MetricsNotice';
import { ResourceSelect } from '@ui/components/ResourceSelect';
import { PillSelect } from '@ui/components/Select';
import { formatCount } from '@ui/lib/format';
import { LineChart, type Point, type Series } from './LineChart';
import { ChartCard, DAY, RANGES, type Range } from './ChartCard';
import styles from './MetricsTab.module.css';
import { MINUTE_MS, HOUR_SECONDS, floorTo } from '@shared/time';

/** Lines that are failures get the error colour. */
const BAD = new Set(['5xx', 'Errors', 'Failed']);

export function formatUnit(unit: MetricUnit, value: number): string {
  switch (unit) {
    case 'count':
      return formatCount(Math.round(value));
    case 'milliseconds':
      return value >= 10_000
        ? `${(value / 1000).toFixed(1)} s`
        : `${Math.round(value).toLocaleString('en-US')} ms`;
    case 'percent':
      return `${value.toFixed(value >= 10 ? 0 : 1)}%`;
    case 'cores':
      return `${value < 0.1 ? value.toFixed(3) : value.toFixed(2)} vCPU`;
    case 'seconds':
      if (value >= HOUR_SECONDS) return `${(value / HOUR_SECONDS).toFixed(1)} h`;
      if (value >= 60) return `${Math.round(value / 60).toLocaleString('en-US')} min`;
      return `${Math.round(value).toLocaleString('en-US')} s`;
    case 'bytes': {
      const units = ['B', 'KB', 'MB', 'GB', 'TB'];
      let size = value;
      let index = 0;
      while (size >= 1000 && index < units.length - 1) {
        size /= 1000;
        index += 1;
      }
      return `${size.toFixed(size >= 10 || index === 0 ? 0 : 1)} ${units[index] ?? 'B'}`;
    }
  }
}

function toPoints(line: ProviderChart['series'][number]): Point[] {
  return line.points.map((point) => ({ t: Date.parse(point.at), v: point.value }));
}

const sum = (points: Point[]) => points.reduce((total, point) => total + point.v, 0);
const mean = (points: Point[]) => (points.length ? sum(points) / points.length : 0);

/** A line's own number over the range, the way its chart sums up. */
function figure(chart: ProviderChart, points: Point[]): number {
  switch (chart.summary) {
    case 'total':
      return sum(points);
    case 'average':
      return mean(points);
    case 'latest':
      return points.at(-1)?.v ?? 0;
  }
}

/** Status classes add up to every request; any other chart leads with its first line. */
export function headline(chart: ProviderChart): number {
  const classes = chart.series.filter((line) => /^\dxx$/.test(line.name));
  if (classes.length > 0) return classes.reduce((all, line) => all + sum(toPoints(line)), 0);
  const first = chart.series[0];
  return first ? figure(chart, toPoints(first)) : 0;
}

const NOTE: Record<ProviderChart['summary'], string> = {
  total: 'in this range',
  average: 'on average',
  latest: 'now',
};

function Card({ chart }: { chart: ProviderChart }) {
  const format = (value: number) => formatUnit(chart.unit, value);
  const tones = ['1', '2', '3'] as const;
  let next = 0;
  const lines: Series[] = [];
  for (const line of chart.series) {
    const tone = BAD.has(line.name) ? 'bad' : tones[next];
    if (!BAD.has(line.name)) next += 1;
    // The palette has three colours and one for errors; further lines are
    // listed beside the chart instead.
    if (tone) lines.push({ name: line.name, tone, points: toPoints(line) });
  }
  const others = chart.series.slice(1);
  return (
    <ChartCard
      label={chart.label}
      value={format(headline(chart))}
      note={NOTE[chart.summary]}
      side={
        others.length > 0 && (
          <ul className={styles.codes} aria-label={`${chart.label} by line`}>
            {chart.series.map((line) => (
              <li key={line.name}>
                <span>{line.name}</span>
                <span>{format(figure(chart, toPoints(line)))}</span>
              </li>
            ))}
          </ul>
        )
      }
    >
      <LineChart label={`${chart.label} over time`} series={lines} format={format} />
    </ChartCard>
  );
}

export function ChartMetrics({
  resources,
  resource,
  onPick,
}: {
  resources: Resource[];
  resource: Resource;
  onPick: (id: string) => void;
}) {
  const [range, setRange] = useState<Range>('24h');
  const chosen = RANGES.find((item) => item.value === range) ?? DAY;
  // Set to the minute when the page opens or the range changes, so going
  // back to a range reuses its read.
  const [end, setEnd] = useState(() => floorTo(Date.now(), MINUTE_MS));
  const metrics = useProviderMetrics(resource.id, {
    from: new Date(end - chosen.span * 1000).toISOString(),
    to: new Date(end).toISOString(),
    step: chosen.step,
  });
  const charts = metrics.data?.charts ?? [];

  return (
    <>
      <div className={styles.controls}>
        <h2 className={styles.title}>Metrics</h2>
        {resources.length > 1 && (
          <ResourceSelect resources={resources} value={resource.id} onChange={onPick} />
        )}
        <PillSelect
          label="Range"
          value={range}
          options={RANGES}
          onChange={(next) => {
            setRange(next);
            setEnd(floorTo(Date.now(), MINUTE_MS));
          }}
        />
      </div>
      {metrics.error && <MetricsNotice provider={resource.provider} error={metrics.error} />}
      {metrics.data?.note && <p className={styles.providerNote}>{metrics.data.note}</p>}
      {metrics.data && charts.length === 0 && (
        <p className={styles.empty}>
          The provider reported no numbers for {resource.name} in this range.
        </p>
      )}
      <div className={styles.grid}>
        {metrics.isPending ? (
          <>
            <ChartCard label="" value="" loading />
            <ChartCard label="" value="" loading />
          </>
        ) : (
          charts.map((chart) => <Card key={chart.id} chart={chart} />)
        )}
      </div>
    </>
  );
}
