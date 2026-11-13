/** Lengths of time, in milliseconds unless the name says seconds. */
export const SECOND_MS = 1_000;
export const MINUTE_MS = 60 * SECOND_MS;
export const HOUR_MS = 60 * MINUTE_MS;
export const DAY_MS = 24 * HOUR_MS;

export const HOUR_SECONDS = 3_600;
export const DAY_SECONDS = 24 * HOUR_SECONDS;

/** Numbers below this are Unix seconds, from above it milliseconds. */
const MILLISECONDS_FROM = 1e12;

/**
 * A provider's timestamp, in milliseconds: providers send Unix seconds,
 * milliseconds or ISO text. NaN when the text isn't a time.
 */
export function providerTime(value: number | string): number {
  if (typeof value === 'string') return Date.parse(value);
  return value < MILLISECONDS_FROM ? value * SECOND_MS : value;
}

/** `time` rounded down to a whole `unit`, such as the start of this minute. */
export function floorTo(time: number, unit: number): number {
  return Math.floor(time / unit) * unit;
}

/** `time` rounded up to a whole `unit`. */
export function ceilTo(time: number, unit: number): number {
  return Math.ceil(time / unit) * unit;
}
