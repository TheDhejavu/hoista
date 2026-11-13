import { describe, expect, it } from 'vitest';
import type { AccountBill } from '@shared/api';
import { billSummary } from './bills';

const bill = (
  provider: AccountBill['provider'],
  cents: number,
  end: string | null,
): AccountBill => ({
  account_id: provider,
  provider,
  label: provider,
  bill: {
    billed: { cents, currency: 'USD' },
    period_start: null,
    period_end: end,
    soft_limit: null,
    hard_limit: null,
    over_limit: false,
  },
});

describe('billSummary', () => {
  it('shows nothing when no provider gives a bill', () => {
    expect(billSummary([])).toBeNull();
    expect(billSummary(undefined)).toBeNull();
  });

  it('adds up only the providers asked for', () => {
    const bills = [bill('railway', 743, '2026-12-01T00:00:00Z'), bill('cloudflare', 500, null)];
    expect(billSummary(bills, ['railway'])).toMatchObject({
      total: { cents: 743 },
      providers: 'Railway',
      ends: '2026-12-01T00:00:00Z',
    });
    expect(billSummary(bills)?.total.cents).toBe(1243);
    expect(billSummary(bills, ['fly'])).toBeNull();
  });
});
