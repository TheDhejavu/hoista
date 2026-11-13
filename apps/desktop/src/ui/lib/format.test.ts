import { describe, expect, it } from 'vitest';
import {
  elapsedClock,
  formatCost,
  formatCount,
  formatDollars,
  formatDuration,
  formatMoney,
  formatPercent,
  hostOf,
  listOf,
  plural,
  timeAgo,
} from './format';

const NOW = Date.parse('2026-11-02T12:00:00Z');

function ago(ms: number): string {
  return new Date(NOW - ms).toISOString();
}

describe('formatCost', () => {
  it('shows a billed amount plainly, and nothing as a dash', () => {
    expect(formatCost({ cents: 500, currency: 'USD' })).toBe('$5.00');
    expect(formatCost({ cents: 0, currency: 'USD' })).toBe('$0.00');
    expect(formatCost(null)).toBe('—');
  });
});

describe('formatMoney', () => {
  it('shows dollars with two decimals', () => {
    expect(formatMoney({ cents: 31, currency: 'USD' })).toBe('$0.31');
    expect(formatMoney({ cents: 1200, currency: 'USD' })).toBe('$12.00');
  });
});

describe('timeAgo', () => {
  it('uses the words from the designs', () => {
    expect(timeAgo(ago(20_000), NOW)).toBe('just now');
    expect(timeAgo(ago(12 * 60_000), NOW)).toBe('12 min ago');
    expect(timeAgo(ago(2 * 3_600_000), NOW)).toBe('2h ago');
    expect(timeAgo(ago(30 * 3_600_000), NOW)).toBe('yesterday');
    expect(timeAgo(ago(9 * 86_400_000), NOW)).toBe('9 days ago');
    expect(timeAgo(ago(33 * 86_400_000), NOW)).toBe('a month ago');
  });
});

describe('small formatters', () => {
  it('formats counts, durations, percents and providers', () => {
    expect(formatCount(12480)).toBe('12,480');
    expect(formatDuration(400)).toBe('0.4s');
    expect(formatDuration(3_000)).toBe('3s');
    expect(formatDuration(38_000)).toBe('38s');
    expect(formatDuration(72_000)).toBe('1m 12s');
    expect(formatPercent(0.002)).toBe('0.2%');
    expect(formatPercent(0.1)).toBe('10%');
    expect(hostOf('https://moodboard.sam.workers.dev/')).toBe('moodboard.sam.workers.dev');
  });
});

describe('words', () => {
  it('pluralises', () => {
    expect(plural(1, 'project')).toBe('1 project');
    expect(plural(3, 'project')).toBe('3 projects');
    expect(plural(2, 'repo is', 'repos are')).toBe('2 repos are');
  });

  it('lists with "and" and no Oxford comma', () => {
    expect(listOf(['Cloudflare'])).toBe('Cloudflare');
    expect(listOf(['Cloudflare', 'Neon'])).toBe('Cloudflare and Neon');
    expect(listOf(['Cloudflare', 'Neon', 'R2'])).toBe('Cloudflare, Neon and R2');
  });

  it('formats whole dollars without cents', () => {
    expect(formatDollars({ cents: 500, currency: 'USD' })).toBe('$5');
    expect(formatDollars({ cents: 750, currency: 'USD' })).toBe('$7.50');
    expect(elapsedClock(65_000)).toBe('01:05');
  });
});
