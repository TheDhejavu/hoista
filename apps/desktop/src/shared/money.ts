/**
 * Money helpers shared by the screens, the adapter and the mock.
 * Amounts are integer cents, never floats.
 */
import type { Money } from './api';

/** `$0.31`, `$12.00`. */
export function formatMoney(money: Money): string {
  return `$${(money.cents / 100).toFixed(2)}`;
}
