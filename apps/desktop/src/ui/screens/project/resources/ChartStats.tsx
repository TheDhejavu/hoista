/**
 * A resource's headline numbers for the last 24 hours, from the charts the
 * engine reads from its provider (requests, emails, CPU…): up to three cards, the
 * same cards the Railway page draws. Nothing shows when the provider measures
 * nothing for it.
 */
import { useState } from 'react';
import type { ProviderChart } from '@engine-api';
import type { Resource } from '@shared/api';
import { providerEntry, serviceName } from '@shared/providers';
import { useCan } from '@ui/api/providers';
import { useProviderMetrics } from '@ui/api/metrics';
import { cx } from '@ui/lib/cx';
import { formatUnit, headline } from '../metrics/ChartMetrics';
import { Bars, Card } from '../overview/ProviderStats';
import overview from '../overview/ProjectOverview.module.css';
import styles from './ResourceScreen.module.css';
import { MetricsNotice } from '../metrics/MetricsNotice';
import { HOUR_MS, DAY_MS, HOUR_SECONDS, ceilTo } from '@shared/time';

const NOTE: Record<ProviderChart['summary'], string> = {
  total: 'in the last 24 hours',
  average: 'on average',
  latest: 'now',
};

const HOUR = new Intl.DateTimeFormat('en-GB', { hour: '2-digit', minute: '2-digit' });

/** Whether the provider draws charts for this resource, above its other numbers. */
export function useCharted(resource: Pick<Resource, 'provider' | 'provider_ref'>): boolean {
  const can = useCan();
  const charts = providerEntry(resource.provider)?.charts;
  return (
    can(resource.provider, 'metrics') &&
    charts !== undefined &&
    (charts.refs ?? ['']).some((ref) => resource.provider_ref.startsWith(ref))
  );
}

export function ChartStats({
  resource,
}: {
  resource: Pick<Resource, 'id' | 'provider' | 'provider_ref'>;
}) {
  const shown = useCharted(resource);
  // Set to the hour when the page opens, so the read is shared.
  const [to] = useState(() => ceilTo(Date.now(), HOUR_MS));
  const metrics = useProviderMetrics(shown ? resource.id : null, {
    from: new Date(to - DAY_MS).toISOString(),
    to: new Date(to).toISOString(),
    step: HOUR_SECONDS,
  });
  if (!shown) return null;
  const provider = serviceName(resource.provider);
  // While the provider is asked, and when it can't answer, one quiet line says
  // so: no empty cards that appear and then vanish.
  if (metrics.isPending) {
    return (
      <p className={styles.metricsNote} role="status">
        <span className={styles.spinner} aria-hidden="true" />
        Reading the last 24 hours from {provider}
      </p>
    );
  }
  if (metrics.error) {
    return <MetricsNotice provider={resource.provider} error={metrics.error} />;
  }
  const list = metrics.data.charts.slice(0, 3);
  if (list.length === 0) return null;
  return (
    <div className={cx(overview.stats)}>
      {list.map((chart) => {
        const format = (value: number) => formatUnit(chart.unit, value);
        // The bars add every line up per hour: all requests, all emails.
        const byTime = new Map<string, number>();
        const lines = chart.series.some((line) => /^\dxx$/.test(line.name))
          ? chart.series.filter((line) => /^\dxx$/.test(line.name))
          : chart.series.slice(0, 1);
        for (const line of lines) {
          for (const point of line.points) {
            byTime.set(point.at, (byTime.get(point.at) ?? 0) + point.value);
          }
        }
        const points = [...byTime.entries()].sort(([a], [b]) => a.localeCompare(b)).slice(-14);
        const labels = points.map(([at]) => HOUR.format(new Date(at)));
        return (
          <Card key={chart.id} label={`${chart.label} · 24h`} value={format(headline(chart))}>
            <div className={overview.small}>{NOTE[chart.summary]}</div>
            {points.length > 1 && (
              <Bars
                values={points.map(([, value]) => value)}
                labels={labels}
                format={format}
                from={labels[0]}
                to="Now"
              />
            )}
          </Card>
        );
      })}
    </div>
  );
}
