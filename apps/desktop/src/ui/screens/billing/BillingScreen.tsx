/**
 * Billing across every connected provider: the total billed so far this period,
 * when the next bill closes and how close each provider is to its limit, then one
 * row per account. Providers that only share usage show it against the plan;
 * providers that share neither say so and link to their own billing page.
 * Only the providers' own numbers, never an estimate.
 */
import { providerEntry, serviceName } from '@shared/providers';
import type { AccountBilling, ProviderPlanUsage, Money } from '@shared/api';
import { Receipt } from 'lucide-react';
import { Link } from 'react-router-dom';
import { useBilling } from '@ui/api/accounts';
import { openExternal } from '@ui/api/system';
import { Button } from '@ui/components/Button';
import { EmptyState } from '@ui/components/EmptyState';
import { Page } from '@ui/components/Page';
import { QueryState } from '@ui/components/QueryState';
import { Skeleton, SkeletonTableBody } from '@ui/components/Skeleton';
import { cx } from '@ui/lib/cx';
import { formatDollars, formatMoney, listOf, plural, shortDate } from '@ui/lib/format';
import { ServiceLogo } from '../accounts/ServiceLogo';
import styles from './BillingScreen.module.css';

export function BillingScreen() {
  const billing = useBilling();
  return (
    <Page>
      <h1 className="visually-hidden">Billing</h1>
      <QueryState query={billing} skeleton={<BillingSkeleton />}>
        {(rows) => <BillingBody rows={rows} />}
      </QueryState>
    </Page>
  );
}

const CARDS = ['Billed so far this period', 'Next period ends', 'Limits', 'Providers reporting'];

/** The page as it will be, with the numbers still on their way. */
function BillingSkeleton() {
  return (
    <div className={styles.skeleton}>
      <div className={styles.stats}>
        {CARDS.map((label) => (
          <div key={label} className={styles.card}>
            <div className={styles.cardLabel}>{label}</div>
            <Skeleton className={styles.cardValueSkeleton} width={96} height={24} />
            <Skeleton width="70%" height={10} />
          </div>
        ))}
      </div>
      <section className={styles.section}>
        <h2 className={styles.sectionTitle}>Billed this period</h2>
        <div className={styles.tableFrame}>
          <table className={styles.table}>
            <thead>
              <tr>
                <th scope="col">Account</th>
                <th scope="col" className={styles.amountHead}>
                  Billed
                </th>
                <th scope="col">Against limit</th>
                <th scope="col">Period</th>
                <th scope="col">
                  <span className="visually-hidden">Actions</span>
                </th>
              </tr>
            </thead>
            <SkeletonTableBody widths={[140, 48, 160, 90, null]} />
          </table>
        </div>
      </section>
    </div>
  );
}

type Billed = AccountBilling & { bill: NonNullable<AccountBilling['bill']> };

function isBilled(row: AccountBilling): row is Billed {
  return row.bill !== null;
}

/** The limit a bill is measured against: the hard cap, else the alert. */
function limitOf(bill: Billed['bill']): Money | null {
  return bill.hard_limit ?? bill.soft_limit;
}

/** How full, from 0 to 1. */
function share(used: number, limit: number): number {
  return limit > 0 ? Math.min(1, used / limit) : 0;
}

function BillingBody({ rows }: { rows: AccountBilling[] }) {
  if (rows.length === 0) {
    return (
      <EmptyState icon={<Receipt />} title="No bills yet">
        Each provider account you connect with <code>hoist accounts add</code>, or on Accounts,
        reports what it billed this period here.
      </EmptyState>
    );
  }

  const billed = rows.filter(isBilled);
  const usage = rows.filter((row) => !row.bill && row.plan_usage.length > 0);
  const neither = rows.filter((row) => !row.bill && row.plan_usage.length === 0);

  return (
    <>
      <Summary rows={rows} billed={billed} usage={usage} />

      {billed.length > 0 && (
        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>Billed this period</h2>
          <div className={styles.tableFrame}>
            <table className={styles.table}>
              <thead>
                <tr>
                  <th scope="col">Account</th>
                  <th scope="col" className={styles.amountHead}>
                    Billed
                  </th>
                  <th scope="col">Against limit</th>
                  <th scope="col">Period</th>
                  <th scope="col">
                    <span className="visually-hidden">Actions</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {billed.map((row) => (
                  <BilledRow key={row.account_id} row={row} />
                ))}
              </tbody>
            </table>
          </div>
        </section>
      )}

      {usage.length > 0 && (
        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>Plan usage</h2>
          <div className={styles.usageGrid}>
            {usage.map((row) => (
              <UsageCard key={row.account_id} row={row} />
            ))}
          </div>
        </section>
      )}

      {neither.length > 0 && (
        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>On the provider only</h2>
          <div className={styles.tableFrame}>
            <table className={styles.table}>
              <tbody>
                {neither.map((row) => (
                  <NoteRow key={row.account_id} row={row} />
                ))}
              </tbody>
            </table>
          </div>
        </section>
      )}

      <p className={styles.footnote}>
        Amounts are what each provider reports through its API. Hoista doesn’t estimate cost; your
        invoice on the provider is the final word.
      </p>
    </>
  );
}

function Summary({
  rows,
  billed,
  usage,
}: {
  rows: AccountBilling[];
  billed: Billed[];
  usage: AccountBilling[];
}) {
  const total: Money = {
    cents: billed.reduce((sum, row) => sum + row.bill.billed.cents, 0),
    currency: 'USD',
  };
  const providers = [...new Set(billed.map((row) => serviceName(row.provider)))];
  const closes = billed
    .map((row) => ({ row, end: row.bill.period_end }))
    .filter((item): item is { row: Billed; end: string } => item.end !== null)
    .sort((a, b) => a.end.localeCompare(b.end));
  const next = closes[0] ?? null;
  const over = billed.filter((row) => row.bill.over_limit);
  // The bill nearest its limit, to show how much room is left.
  const nearest = billed
    .map((row) => ({ row, limit: limitOf(row.bill) }))
    .filter((item): item is { row: Billed; limit: Money } => item.limit !== null)
    .sort(
      (a, b) =>
        share(b.row.bill.billed.cents, b.limit.cents) -
        share(a.row.bill.billed.cents, a.limit.cents),
    )[0];

  return (
    <div className={styles.stats}>
      <div className={styles.card}>
        <div className={styles.cardLabel}>Billed so far this period</div>
        <div className={styles.cardValue}>{formatMoney(total)}</div>
        <div className={styles.small}>
          {providers.length > 0
            ? `Across ${listOf(providers)}`
            : 'No connected provider shares a bill'}
        </div>
      </div>

      <div className={styles.card}>
        <div className={styles.cardLabel}>Next period ends</div>
        <div className={styles.cardValue}>{next ? shortDate(next.end) : '—'}</div>
        <div className={styles.small}>
          {next
            ? `${serviceName(next.row.provider)} · ${formatMoney(next.row.bill.billed)} so far`
            : 'No provider gave a period end'}
        </div>
      </div>

      <div className={styles.card}>
        <div className={styles.cardLabel}>Limits</div>
        <div className={cx(styles.cardValue, over.length > 0 && styles.bad)}>
          {over.length > 0
            ? `Over on ${plural(over.length, 'provider')}`
            : nearest
              ? 'Within limits'
              : 'None set'}
        </div>
        {nearest ? (
          <>
            <div className={styles.small}>
              {`${serviceName(nearest.row.provider)}: ${formatDollars(nearest.row.bill.billed)} of ${formatDollars(nearest.limit)}`}
            </div>
            <Meter value={share(nearest.row.bill.billed.cents, nearest.limit.cents)} />
          </>
        ) : (
          <div className={styles.small}>Set spend limits on the provider</div>
        )}
      </div>

      <div className={styles.card}>
        <div className={styles.cardLabel}>Providers reporting</div>
        <div className={styles.cardValue}>
          {billed.length + usage.length} of {rows.length}
        </div>
        <div className={styles.small}>
          {`${plural(billed.length, 'bill')} · ${usage.length} with plan usage`}
        </div>
      </div>
    </div>
  );
}

function Meter({ value, bad = false }: { value: number; bad?: boolean }) {
  const tone = bad || value >= 1 ? styles.fillBad : value >= 0.8 ? styles.fillWarn : undefined;
  return (
    <div
      className={styles.track}
      role="meter"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(value * 100)}
    >
      <div className={cx(styles.fill, tone)} style={{ width: `${Math.max(2, value * 100)}%` }} />
    </div>
  );
}

function AccountCell({ row }: { row: AccountBilling }) {
  const name = serviceName(row.provider);
  return (
    <span className={styles.accountCell}>
      <ServiceLogo id={row.provider} name={name} />
      <span className={styles.accountName}>
        <Link to={`/accounts/${row.account_id}`} className={styles.name}>
          {name}
        </Link>
        <span className={styles.identity}>{row.label}</span>
      </span>
    </span>
  );
}

function BillingButton({ row }: { row: AccountBilling }) {
  const page = providerEntry(row.provider)?.billing ?? null;
  if (!page) return null;
  return (
    <Button
      onClick={() => {
        void openExternal(page);
      }}
    >
      Open billing ↗
    </Button>
  );
}

function BilledRow({ row }: { row: Billed }) {
  const { bill } = row;
  const limit = limitOf(bill);
  return (
    <tr>
      <td>
        <AccountCell row={row} />
      </td>
      <td className={cx(styles.amount, bill.over_limit && styles.bad)}>
        {formatMoney(bill.billed)}
      </td>
      <td>
        {limit ? (
          <span className={styles.limitCell}>
            <Meter value={share(bill.billed.cents, limit.cents)} bad={bill.over_limit} />
            <span className={styles.muted}>
              {`${formatDollars(limit)} ${bill.hard_limit ? 'cap' : 'alert'}`}
            </span>
          </span>
        ) : (
          <span className={styles.muted}>No limit set</span>
        )}
      </td>
      <td className={styles.muted}>
        {bill.period_start
          ? bill.period_end
            ? `${shortDate(bill.period_start)} – ${shortDate(bill.period_end)}`
            : `Since ${shortDate(bill.period_start)}`
          : '—'}
      </td>
      <td className={styles.actionCell}>
        <BillingButton row={row} />
      </td>
    </tr>
  );
}

/** `41.6`, `1,200`. */
function amount(value: number): string {
  return value.toLocaleString('en-US', { maximumFractionDigits: 2 });
}

function UsageLine({ usage }: { usage: ProviderPlanUsage }) {
  return (
    <div className={styles.usageLine}>
      <div className={styles.usageTop}>
        <span>{usage.label}</span>
        <span className={styles.muted}>
          {usage.limit === null
            ? `${amount(usage.used)} ${usage.unit}`
            : `${amount(usage.used)} of ${amount(usage.limit)} ${usage.unit}`}
        </span>
      </div>
      {usage.limit !== null && <Meter value={share(usage.used, usage.limit)} />}
    </div>
  );
}

function UsageCard({ row }: { row: AccountBilling }) {
  return (
    <div className={styles.card}>
      <div className={styles.usageHead}>
        <AccountCell row={row} />
        <BillingButton row={row} />
      </div>
      <div className={styles.usageLines}>
        {row.plan_usage.map((usage) => (
          <UsageLine key={usage.label} usage={usage} />
        ))}
      </div>
    </div>
  );
}

function NoteRow({ row }: { row: AccountBilling }) {
  return (
    <tr>
      <td className={styles.noteAccount}>
        <AccountCell row={row} />
      </td>
      <td className={styles.note}>{row.note ?? 'Nothing to show yet.'}</td>
      <td className={styles.actionCell}>
        <BillingButton row={row} />
      </td>
    </tr>
  );
}
