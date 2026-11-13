/**
 * Formatting for numbers, money and times, in the wording the designs use.
 */
import type { Money, Timestamp } from '@shared/api';
import { formatMoney } from '@shared/money';
import { DAY_MS, HOUR_MS, MINUTE_MS, SECOND_MS } from '@shared/time';

export { formatMoney };

/**
 * A billed amount: `$5.00`. Hoista shows only what a provider actually billed,
 * never an estimate. Null means there is no bill to show: `—`.
 */
export function formatCost(money: Money | null): string {
  return money ? formatMoney(money) : '—';
}

/** `Nov 30`, for a billing period's end. */
export function shortDate(at: Timestamp): string {
  return new Date(at).toLocaleDateString('en-US', {
    month: 'short',
    day: 'numeric',
    timeZone: 'UTC',
  });
}

/** `$5` for whole dollars, `$7.50` otherwise: for bills and limits. */
export function formatDollars(money: Money): string {
  return money.cents % 100 === 0 ? `$${money.cents / 100}` : formatMoney(money);
}

/** `12,480`. */
export function formatCount(value: number): string {
  return value.toLocaleString('en-US');
}

/** `just now`, `12 min ago`, `2h ago`, `yesterday`, `9 days ago`, `a month ago`. */
export function timeAgo(at: Timestamp, now: number = Date.now()): string {
  const when = Date.parse(at);
  // A time the provider wrote in a shape that can't be read shows nothing, never "NaN".
  if (Number.isNaN(when)) return '—';
  const elapsed = Math.max(0, now - when);

  if (elapsed < MINUTE_MS) return 'just now';
  if (elapsed < HOUR_MS) return `${Math.floor(elapsed / MINUTE_MS)} min ago`;
  if (elapsed < DAY_MS) return `${Math.floor(elapsed / HOUR_MS)}h ago`;

  const days = Math.floor(elapsed / DAY_MS);
  if (days === 1) return 'yesterday';
  if (days < 30) return `${days} days ago`;
  if (days < 60) return 'a month ago';
  return `${Math.floor(days / 30)} months ago`;
}

/** The short form for tables: `40m ago`, `2h ago`, `3d ago`, `2mo ago`. */
export function shortAgo(at: Timestamp, now: number = Date.now()): string {
  const elapsed = Math.max(0, now - Date.parse(at));

  if (elapsed < MINUTE_MS) return 'just now';
  if (elapsed < HOUR_MS) return `${Math.floor(elapsed / MINUTE_MS)}m ago`;
  if (elapsed < DAY_MS) return `${Math.floor(elapsed / HOUR_MS)}h ago`;
  const days = Math.floor(elapsed / DAY_MS);
  if (days < 30) return `${days}d ago`;
  return `${Math.floor(days / 30)}mo ago`;
}

/** `0.4s`, `3.2s`, `38s`, `1m 12s`. */
export function formatDuration(ms: number): string {
  // Short steps keep a decimal so 0.4s doesn't read as 0s.
  if (ms < 10 * SECOND_MS) return `${(ms / SECOND_MS).toFixed(1).replace(/\.0$/, '')}s`;
  const seconds = Math.round(ms / SECOND_MS);
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}

/** `0.2%` from a 0–1 ratio. */
export function formatPercent(ratio: number): string {
  return `${(ratio * 100).toFixed(1).replace(/\.0$/, '')}%`;
}

/** `moodboard.sam.workers.dev` from `https://moodboard.sam.workers.dev`. */
export function hostOf(url: string): string {
  return url.replace(/^https?:\/\//, '').replace(/\/$/, '');
}

/** `1 project`, `3 projects`. Pass `pluralWord` for irregular words: `plural(2, 'is', 'are')`. */
export function plural(count: number, word: string, pluralWord = `${word}s`): string {
  return `${count} ${count === 1 ? word : pluralWord}`;
}

const AND_LIST = new Intl.ListFormat('en', { style: 'long', type: 'conjunction' });

/** `Cloudflare`, `Cloudflare and Neon`, `Cloudflare, Neon and R2`. */
export function listOf(items: readonly string[]): string {
  return AND_LIST.format(items).replace(/, and /, ' and ');
}

/** `01:05` minutes and seconds since a deploy started, for build output. */
export function elapsedClock(ms: number): string {
  const seconds = Math.floor(ms / SECOND_MS);
  const minutes = Math.floor(seconds / 60);
  return `${String(minutes).padStart(2, '0')}:${String(seconds % 60).padStart(2, '0')}`;
}

/** `deployed v7 on short-links`: what an agent did, to follow its name in a sentence. */
export function activityText(summary: string, projectId: string | null): string {
  const text = summary.charAt(0).toLowerCase() + summary.slice(1);
  return projectId && !summary.includes(projectId) ? `${text} on ${projectId}` : text;
}

/** A size given in gigabytes, as `512 MB`, `2.1 GB` or `24 GB`. */
export function formatGigabytes(gb: number): string {
  if (gb < 1) return `${String(Math.round(gb * 1000))} MB`;
  return `${gb.toFixed(gb >= 10 ? 0 : 1)} GB`;
}

/** A size given in megabytes, as `512 MB` or `2.1 GB`. */
export function formatMegabytes(mb: number): string {
  return mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : `${String(Math.round(mb))} MB`;
}
