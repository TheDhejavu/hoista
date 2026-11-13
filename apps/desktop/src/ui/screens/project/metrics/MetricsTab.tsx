/**
 * A watched resource's numbers over time, at `/projects/:appId/metrics`, as its
 * provider reports them: requests, errors by status code, response time, CPU
 * and memory against their limits, network and disk. Nothing is estimated;
 * a chart the provider has no numbers for is left out. Agents read the same
 * numbers with `get_metrics`.
 */
import { runsCode } from '../resourceKinds';
import { useCan } from '@ui/api/providers';
import { useMemo, useState, type ReactNode } from 'react';
import type { ProviderTarget, Resource } from '@shared/api';
import { useProject } from '@ui/api/projects';
import { providerEntry } from '@shared/providers';
import { useProviderRead } from '@ui/api/providerOperations';
import { QueryState } from '@ui/components/QueryState';
import { Skeleton } from '@ui/components/Skeleton';
import { PillSelect } from '@ui/components/Select';
import { formatCount, formatGigabytes } from '@ui/lib/format';
import { useRequiredParam } from '@ui/lib/useRequiredParam';
import { ProjectTab } from '../ProjectTab';
import { LineChart, type Point } from './LineChart';
import styles from './MetricsTab.module.css';
import { ResourceSelect } from '@ui/components/ResourceSelect';
import { ChartMetrics } from './ChartMetrics';
import { ChartCard, DAY, RANGES, type Range } from './ChartCard';
import { MINUTE_MS, floorTo, providerTime } from '@shared/time';

interface Sample {
  ts: number | string;
  value: number;
}
type ByStatus = { statusCode: number; samples: Sample[] }[];
type Durations = {
  samples: { ts: number | string; p50: number | null; p95: number | null; p99: number | null }[];
};
type Measured = { measurement: string; values: Sample[] }[];

function points(samples: Sample[] | undefined): Point[] {
  return (samples ?? []).map((sample) => ({ t: providerTime(sample.ts), v: sample.value }));
}

function total(list: Point[]): number {
  return list.reduce((sum, point) => sum + point.v, 0);
}

function average(list: Point[]): number | null {
  return list.length ? total(list) / list.length : null;
}

const ms = (value: number) => `${Math.round(value).toLocaleString('en-US')} ms`;
const gb = formatGigabytes;
const vcpu = (value: number) => `${value < 0.1 ? value.toFixed(3) : value.toFixed(2)} vCPU`;

/** `pinned` shows one resource's charts only, on that resource's page. */
export function MetricsTab({ pinned }: { pinned?: string } = {}) {
  const projectId = useRequiredParam('projectId');
  const project = useProject(projectId);
  const can = useCan();
  return (
    <ProjectTab>
      <QueryState query={project} skeleton={<MetricsSkeleton />}>
        {(detail) => (
          <Metrics
            resources={detail.resources.filter(
              (resource) =>
                (pinned === undefined || resource.id === pinned) &&
                can(resource.provider, 'metrics') &&
                (providerEntry(resource.provider)?.charts?.refs ?? ['']).some((ref) =>
                  resource.provider_ref.startsWith(ref),
                ),
            )}
          />
        )}
      </QueryState>
    </ProjectTab>
  );
}

/** The charts' cards, empty, while the project loads. */
function MetricsSkeleton() {
  return (
    <>
      <div className={styles.controls}>
        <Skeleton width={160} height={30} />
        <Skeleton width={140} height={30} />
      </div>
      <div className={styles.grid}>
        {[0, 1, 2, 3].map((card) => (
          <section key={card} className={styles.card}>
            <span className={styles.cardSkeleton}>
              <Skeleton width={80} height={10} />
              <Skeleton width={90} height={22} />
            </span>
            <Skeleton height={164} className={styles.chartSkeleton} />
          </section>
        ))}
      </div>
    </>
  );
}

function Metrics({ resources }: { resources: Resource[] }) {
  const [chosenId, setChosenId] = useState<string | null>(null);
  const chosen =
    resources.find((item) => item.id === chosenId) ??
    resources.find((item) => runsCode(item.kind)) ??
    resources[0];
  // Most providers answer with charts the engine reads for them.
  if (chosen && providerEntry(chosen.provider)?.charts) {
    return (
      <ChartMetrics key={chosen.id} resources={resources} resource={chosen} onPick={setChosenId} />
    );
  }
  // A provider that reports its own request counts (Supabase) draws those.
  if (chosen && providerEntry(chosen.provider)?.usageSeries) {
    return <UsageMetrics resources={resources} resource={chosen} onPick={setChosenId} />;
  }
  return (
    <ProviderMetrics
      key={chosen?.id}
      resources={resources}
      first={chosen?.id ?? null}
      onPick={setChosenId}
    />
  );
}

function ProviderMetrics({
  resources,
  first,
  onPick,
}: {
  resources: Resource[];
  first: string | null;
  onPick: (id: string) => void;
}) {
  const [resourceId, setResourceId] = useState<string | null>(first);
  const [range, setRange] = useState<Range>('24h');
  const resource = resources.find((item) => item.id === resourceId) ?? resources[0];
  const chosen = RANGES.find((item) => item.value === range) ?? DAY;
  // The window is set to the minute when the page opens or the range changes,
  // so the reads behind every chart are shared.
  const [end, setEnd] = useState(() => floorTo(Date.now(), MINUTE_MS));
  const window = useMemo(
    () => ({
      startDate: new Date(end - chosen.span * 1000).toISOString(),
      endDate: new Date(end).toISOString(),
    }),
    [end, chosen.span],
  );
  const target: ProviderTarget | null = resource ? { kind: 'resource', id: resource.id } : null;
  const http = { ...window, stepSeconds: chosen.step };
  const byStatus = useProviderRead<ByStatus>(target, 'httpMetricsGroupedByStatus', http);
  const durations = useProviderRead<Durations>(target, 'httpDurationMetrics', http);
  const usage = useProviderRead<Measured>(target, 'metrics', {
    ...window,
    sampleRateSeconds: chosen.step,
    measurements: [
      'CPU_USAGE',
      'CPU_LIMIT',
      'MEMORY_USAGE_GB',
      'MEMORY_LIMIT_GB',
      'NETWORK_RX_GB',
      'NETWORK_TX_GB',
      'DISK_USAGE_GB',
    ],
  });

  if (!resource) {
    return (
      <p className={styles.empty}>This project has nothing on a provider that reports metrics.</p>
    );
  }

  // Requests: every status summed per sample; errors: 5xx summed.
  const sumBy = (codes: ByStatus) => {
    const byTime = new Map<number, number>();
    for (const status of codes) {
      for (const point of points(status.samples)) {
        byTime.set(point.t, (byTime.get(point.t) ?? 0) + point.v);
      }
    }
    return [...byTime.entries()].sort(([a], [b]) => a - b).map(([t, v]) => ({ t, v }));
  };
  const statuses = byStatus.data ?? [];
  const requests = sumBy(statuses);
  const serverErrors = sumBy(statuses.filter((status) => status.statusCode >= 500));
  const codes = statuses
    .map((status) => ({ code: status.statusCode, count: total(points(status.samples)) }))
    .filter((item) => item.count > 0)
    .sort((a, b) => b.count - a.count);

  const percentile = (key: 'p50' | 'p95' | 'p99') =>
    (durations.data?.samples ?? [])
      .filter((sample) => sample[key] !== null)
      .map((sample) => ({ t: providerTime(sample.ts), v: sample[key] ?? 0 }));
  const p50 = percentile('p50');
  const p95 = percentile('p95');
  const p99 = percentile('p99');

  const measured = (name: string) =>
    points((usage.data ?? []).find((item) => item.measurement === name)?.values);
  const cpu = measured('CPU_USAGE');
  const cpuLimit = measured('CPU_LIMIT');
  const memory = measured('MEMORY_USAGE_GB');
  const memoryLimit = measured('MEMORY_LIMIT_GB');
  const received = measured('NETWORK_RX_GB');
  const sent = measured('NETWORK_TX_GB');
  const disk = measured('DISK_USAGE_GB');

  const loading = byStatus.isPending || durations.isPending || usage.isPending;
  const cards: ReactNode[] = [];

  if (byStatus.isPending) {
    cards.push(
      <ChartCard key="requests" label="Requests" value="" loading />,
      <ChartCard key="errors" label="Server errors (5xx)" value="" loading />,
    );
  }
  if (byStatus.data) {
    cards.push(
      <ChartCard key="requests" label="Requests" value={formatCount(total(requests))}>
        <LineChart
          label="Requests over time"
          series={[{ name: 'Requests', tone: '1', points: requests }]}
          format={(value) => formatCount(Math.round(value))}
        />
      </ChartCard>,
      <ChartCard
        key="errors"
        label="Server errors (5xx)"
        value={formatCount(total(serverErrors))}
        side={
          codes.length > 0 && (
            <ul className={styles.codes} aria-label="Responses by status code">
              <li className={styles.codesHead}>
                <span>Status</span>
                <span>Responses</span>
              </li>
              {codes.slice(0, 6).map((item) => (
                <li key={item.code}>
                  <span className="mono">{item.code}</span>
                  <span>{formatCount(item.count)}</span>
                </li>
              ))}
            </ul>
          )
        }
      >
        <LineChart
          label="Server errors over time"
          empty="No server errors in this range."
          series={[{ name: '5xx', tone: 'bad', points: serverErrors }]}
          format={(value) => formatCount(Math.round(value))}
        />
      </ChartCard>,
    );
  }
  if (durations.isPending) {
    cards.push(<ChartCard key="latency" label="Response time" value="" loading />);
  }
  if (p50.length > 0) {
    cards.push(
      <ChartCard
        key="latency"
        label="Response time"
        value={ms(p50.at(-1)?.v ?? 0)}
        note={`p50 now · p95 ${ms(p95.at(-1)?.v ?? 0)} · p99 ${ms(p99.at(-1)?.v ?? 0)}`}
      >
        <LineChart
          label="Response time percentiles over time"
          series={[
            { name: 'p50', tone: '1', points: p50 },
            { name: 'p95', tone: '2', points: p95 },
            { name: 'p99', tone: '3', points: p99 },
          ]}
          format={ms}
        />
      </ChartCard>,
    );
  }
  if (usage.isPending) {
    cards.push(
      <ChartCard key="cpu" label="CPU" value="" loading />,
      <ChartCard key="memory" label="Memory" value="" loading />,
      <ChartCard key="network" label="Network" value="" loading />,
      <ChartCard key="disk" label="Disk" value="" loading />,
    );
  }
  if (cpu.length > 0) {
    const limit = cpuLimit.at(-1)?.v;
    cards.push(
      <ChartCard
        key="cpu"
        label="CPU"
        value={vcpu(average(cpu) ?? 0)}
        note={`on average${limit ? ` · limit ${String(limit)} vCPU` : ''}`}
      >
        <LineChart
          label="CPU use over time"
          series={[{ name: 'CPU', tone: '1', points: cpu }]}
          format={vcpu}
        />
      </ChartCard>,
    );
  }
  if (memory.length > 0) {
    const limit = memoryLimit.at(-1)?.v;
    cards.push(
      <ChartCard
        key="memory"
        label="Memory"
        value={gb(average(memory) ?? 0)}
        note={`on average${limit ? ` · limit ${gb(limit)}` : ''}`}
      >
        <LineChart
          label="Memory use over time"
          series={[{ name: 'Memory', tone: '1', points: memory }]}
          format={gb}
        />
      </ChartCard>,
    );
  }
  if (received.length > 0 || sent.length > 0) {
    cards.push(
      <ChartCard
        key="network"
        label="Network"
        value={`${gb(total(received))} in`}
        note={`${gb(total(sent))} out`}
      >
        <LineChart
          label="Network in and out over time"
          series={[
            { name: 'In', tone: '1', points: received },
            { name: 'Out', tone: '2', points: sent },
          ]}
          format={gb}
        />
      </ChartCard>,
    );
  }
  if (disk.length > 0) {
    cards.push(
      <ChartCard key="disk" label="Disk" value={gb(disk.at(-1)?.v ?? 0)} note="used now">
        <LineChart
          label="Disk use over time"
          series={[{ name: 'Disk', tone: '1', points: disk }]}
          format={gb}
        />
      </ChartCard>,
    );
  }

  return (
    <>
      <div className={styles.controls}>
        <h2 className={styles.title}>Metrics</h2>
        {resources.length > 1 && (
          <ResourceSelect
            resources={resources}
            value={resource.id}
            onChange={(id: string) => {
              setResourceId(id);
              onPick(id);
            }}
          />
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
      {!loading && cards.length === 0 && (
        <p className={styles.empty}>
          The provider reported no numbers for {resource.name} in this range.
        </p>
      )}
      <div className={styles.grid}>{cards}</div>
    </>
  );
}

/**
 * Metrics for a provider that counts requests per service itself: the counts
 * over time, by service, and how full the disk is. Ranges it can't answer
 * aren't offered.
 */
function UsageMetrics({
  resources,
  resource,
  onPick,
}: {
  resources: Resource[];
  resource: Resource;
  onPick: (id: string) => void;
}) {
  const spec = providerEntry(resource.provider)?.usageSeries;
  const ranges = RANGES.filter((item) => spec?.intervals[item.value]);
  const [range, setRange] = useState<Range>(
    ranges.find((item) => item.value === '24h')?.value ?? '1h',
  );
  const target: ProviderTarget = { kind: 'resource', id: resource.id };
  const usage = useProviderRead<Record<string, unknown>>(target, spec?.op ?? '', {
    interval: spec?.intervals[range],
  });
  const disk = useProviderRead<{ metrics?: { fs_used_bytes?: number; fs_size_bytes?: number } }>(
    spec?.disk ? target : null,
    spec?.disk ?? '',
  );
  const rows = ((spec && usage.data?.[spec.field]) ?? []) as Record<string, unknown>[];
  const at = (row: Record<string, unknown>) => Date.parse(String(row.timestamp));
  const series = Object.entries(spec?.series ?? {}).map(([field, name]) => ({
    name,
    points: rows.map((row) => ({ t: at(row), v: Number(row[field] ?? 0) })),
  }));
  const totals = series
    .map((item) => ({ name: item.name, count: total(item.points) }))
    .sort((a, b) => b.count - a.count);
  const all = rows.map((row, i) => ({
    t: at(row),
    v: series.reduce((sum, item) => sum + (item.points[i]?.v ?? 0), 0),
  }));
  const tones = ['1', '2', '3'] as const;
  const used = disk.data?.metrics?.fs_used_bytes;
  const size = disk.data?.metrics?.fs_size_bytes;
  const toGb = (bytes: number) => bytes / 1_000_000_000;

  return (
    <>
      <div className={styles.controls}>
        <h2 className={styles.title}>Metrics</h2>
        {resources.length > 1 && (
          <ResourceSelect resources={resources} value={resource.id} onChange={onPick} />
        )}
        <PillSelect label="Range" value={range} options={ranges} onChange={setRange} />
      </div>
      <div className={styles.grid}>
        {usage.isPending ? (
          <ChartCard label="Requests" value="" loading />
        ) : (
          <ChartCard
            label="Requests"
            value={formatCount(total(all))}
            // The provider counts these for its whole project, whichever
            // resource of it is picked: say so, so the numbers don't seem
            // to be the picked resource's alone.
            note={`for the whole ${providerEntry(resource.provider)?.name ?? 'provider'} project ${resource.name} is in · its own count`}
            side={
              <ul className={styles.codes} aria-label="Requests by service">
                {totals.map((item) => (
                  <li key={item.name}>
                    <span>{item.name}</span>
                    <span>{formatCount(item.count)}</span>
                  </li>
                ))}
              </ul>
            }
          >
            <LineChart
              label="Requests by service over time"
              series={series.slice(0, 3).map((item, i) => ({ ...item, tone: tones[i] ?? '1' }))}
              format={(value) => formatCount(Math.round(value))}
            />
          </ChartCard>
        )}
        {used !== undefined && size !== undefined && size > 0 && (
          <ChartCard
            label="Disk"
            value={gb(toGb(used))}
            note={`of ${gb(toGb(size))} · ${String(Math.round((used / size) * 100))}% used`}
          />
        )}
      </div>
    </>
  );
}
