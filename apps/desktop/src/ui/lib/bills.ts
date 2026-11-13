/**
 * What providers have actually billed, summed for display. Hoista shows only a
 * provider's own bill, per account, never an estimate and never a per-project share.
 */
import type { AccountBill, Money, ProviderId } from '@shared/api';
import { serviceName } from '@shared/providers';

interface BillSummary {
  /** Sum of every bill shown. */
  total: Money;
  /** `Railway`, or `Railway and Cloudflare`. */
  providers: string;
  /** The earliest period end, when the providers say. */
  ends: string | null;
  over_limit: boolean;
}

export function billSummary(
  bills: readonly AccountBill[] | undefined,
  providers?: readonly ProviderId[],
): BillSummary | null {
  const shown = (bills ?? []).filter((item) => !providers || providers.includes(item.provider));
  if (shown.length === 0) return null;
  const names = [...new Set(shown.map((item) => serviceName(item.provider)))];
  const ends = shown
    .map((item) => item.bill.period_end)
    .filter((end): end is string => end !== null)
    .sort()[0];
  return {
    total: {
      cents: shown.reduce((sum, item) => sum + item.bill.billed.cents, 0),
      currency: shown[0]?.bill.billed.currency ?? 'USD',
    },
    providers:
      names.length > 1 ? `${names.slice(0, -1).join(', ')} and ${names.at(-1)}` : (names[0] ?? ''),
    ends: ends ?? null,
    over_limit: shown.some((item) => item.bill.over_limit),
  };
}
