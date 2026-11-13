/**
 * A small line chart: one shared x (time), one y axis from zero, thin lines,
 * a recessive grid, and a crosshair that snaps to the nearest sample and
 * lists every series there. A dashed series is a ceiling, such as a limit.
 */
import { useRef, useState } from 'react';
import { cx } from '@ui/lib/cx';
import styles from './MetricsTab.module.css';
import { HOUR_MS, DAY_MS } from '@shared/time';

export interface Point {
  t: number;
  v: number;
}

export interface Series {
  name: string;
  /** A slot from the chart palette: 1–3, or `bad` for errors, `limit` for a ceiling. */
  tone: '1' | '2' | '3' | 'bad' | 'limit';
  points: Point[];
}

const W = 600;
const H = 160;

function timeLabel(t: number, spanMs: number): string {
  const day = new Date(t);
  const clock = day.toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' });
  if (spanMs <= 3 * HOUR_MS) return clock;
  if (spanMs <= 2 * DAY_MS) {
    return `${day.toLocaleDateString('en-GB', { day: 'numeric', month: 'short' })} ${clock}`;
  }
  return day.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
}

export function LineChart({
  series,
  format,
  label,
  empty = 'No samples in this range.',
}: {
  series: Series[];
  format: (value: number) => string;
  /** What the chart shows, for screen readers. */
  label: string;
  /** What to say when there's nothing to draw. */
  empty?: string;
}) {
  const box = useRef<HTMLDivElement>(null);
  const [hover, setHover] = useState<number | null>(null);
  const times = [...new Set(series.flatMap((item) => item.points.map((point) => point.t)))].sort(
    (a, b) => a - b,
  );
  const first = times[0];
  const last = times.at(-1);
  if (first === undefined || last === undefined) {
    return <div className={styles.noData}>{empty}</div>;
  }
  const span = Math.max(last - first, 1);
  const top = Math.max(...series.flatMap((item) => item.points.map((point) => point.v)), 0) * 1.15;
  const max = top > 0 ? top : 1;
  const x = (t: number) => ((t - first) / span) * W;
  const y = (v: number) => H - (v / max) * H;
  const at = hover === null ? null : times[hover];

  return (
    <div className={styles.chart}>
      <div className={styles.yMax}>{format(max)}</div>
      <div
        ref={box}
        className={styles.plot}
        role="img"
        aria-label={label}
        onPointerMove={(event) => {
          const rect = box.current?.getBoundingClientRect();
          if (!rect) return;
          const t = first + ((event.clientX - rect.left) / rect.width) * span;
          let best = 0;
          times.forEach((time, index) => {
            if (Math.abs(time - t) < Math.abs((times[best] ?? 0) - t)) best = index;
          });
          setHover(best);
        }}
        onPointerLeave={() => {
          setHover(null);
        }}
      >
        <svg viewBox={`0 0 ${String(W)} ${String(H)}`} preserveAspectRatio="none">
          {[0.25, 0.5, 0.75].map((f) => (
            <line key={f} x1={0} x2={W} y1={H * f} y2={H * f} className={styles.grid} />
          ))}
          <line x1={0} x2={W} y1={H} y2={H} className={styles.axis} />
          {series.map((item) => (
            <polyline
              key={item.name}
              className={cx(styles.line, styles[`tone${item.tone}`])}
              points={item.points
                .map((point) => `${x(point.t).toFixed(1)},${y(point.v).toFixed(1)}`)
                .join(' ')}
            />
          ))}
        </svg>
        {at !== null && at !== undefined && (
          <>
            <div className={styles.crosshair} style={{ left: `${String((x(at) / W) * 100)}%` }} />
            <div
              className={cx(styles.tip, x(at) > W / 2 && styles.tipLeft)}
              style={{ left: `${String((x(at) / W) * 100)}%` }}
            >
              <div className={styles.tipTime}>
                {new Date(at).toLocaleString('en-GB', {
                  day: 'numeric',
                  month: 'short',
                  hour: '2-digit',
                  minute: '2-digit',
                })}
              </div>
              {series.map((item) => {
                const point = item.points.find((p) => p.t === at);
                return (
                  <div key={item.name} className={styles.tipRow}>
                    <span className={cx(styles.swatch, styles[`tone${item.tone}`])} />
                    <b>{point ? format(point.v) : '—'}</b>
                    <span>{item.name}</span>
                  </div>
                );
              })}
            </div>
          </>
        )}
      </div>
      <div className={styles.xAxis}>
        <span>{timeLabel(first, span)}</span>
        <span>{timeLabel(first + span / 2, span)}</span>
        <span>{timeLabel(last, span)}</span>
      </div>
      {series.length > 1 && (
        <div className={styles.legend}>
          {series.map((item) => (
            <span key={item.name}>
              <span className={cx(styles.swatch, styles[`tone${item.tone}`])} />
              {item.name}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}
