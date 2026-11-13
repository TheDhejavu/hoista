import { serviceName } from '@shared/providers';
import type { Account, AccountBill } from '@shared/api';
import { useRunProviderOperation } from '@ui/api/providerOperations';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import type { Option } from '@ui/components/Select';
import { SelectRow, SettingGroup } from '@ui/components/SettingRow';
import { showToast } from '@ui/lib/toast';
import { formatCost, shortDate } from '@ui/lib/format';
import styles from './BillSection.module.css';
import shared from './account.module.css';
import { dollars } from './accountWords';

/** Whole dollars, for the provider's limits; `null` is not set. */
const SOFT_LIMITS: Option<number | null>[] = [
  { value: null, label: 'Not set' },
  ...[5, 10, 20, 50, 100, 200].map((dollars) => ({ value: dollars, label: `$${String(dollars)}` })),
];

export function BillSection({ account, bill }: { account: Account; bill: AccountBill }) {
  const name = serviceName(account.service);
  const run = useRunProviderOperation({ kind: 'account', id: account.id });
  const { billed, soft_limit: soft, hard_limit: hard, customer_id: customer } = bill.bill;
  const softDollars = dollars(soft);
  const hardDollars = dollars(hard);
  // Railway stops services at the hard limit; it can't be below the email.
  const hardOptions: Option<number | null>[] = [
    { value: null, label: 'No limit' },
    ...[10, 20, 50, 100, 200, 500].map((amount) => ({
      value: amount,
      label: `$${String(amount)}`,
      disabled: softDollars === null || amount < softDollars,
    })),
  ];
  const scale = Math.max(hardDollars ?? 0, softDollars ?? 0, dollars(billed) ?? 0, 1);
  const share = (amount: number | null) =>
    `${String(Math.min(100, ((amount ?? 0) / scale) * 100))}%`;

  const setLimits = (nextSoft: number | null, nextHard: number | null) => {
    if (!customer) return;
    const saved = {
      onSuccess: () => {
        showToast(
          nextSoft === null
            ? `Limits removed at ${name}`
            : `Saved at ${name}: email at $${String(nextSoft)}${nextHard === null ? '' : `, stop at $${String(Math.max(nextHard, nextSoft))}`}`,
        );
      },
    };
    if (nextSoft === null) {
      run.mutate({ name: 'usageLimitRemove', args: { input: { customerId: customer } } }, saved);
      return;
    }
    run.mutate(
      {
        name: 'usageLimitSet',
        args: {
          input: {
            customerId: customer,
            softLimitDollars: nextSoft,
            ...(nextHard === null ? {} : { hardLimitDollars: Math.max(nextHard, nextSoft) }),
          },
        },
      },
      saved,
    );
  };

  return (
    <>
      <section className={styles.billCard} aria-label="Used so far">
        <div className={styles.billLabel}>
          Used so far
          {bill.bill.period_start && bill.bill.period_end
            ? ` · ${shortDate(bill.bill.period_start)} – ${shortDate(bill.bill.period_end)}`
            : ''}
        </div>
        <div className={styles.billRow}>
          <span className={styles.billAmount}>{formatCost(billed)}</span>
          {bill.bill.over_limit && <span className={styles.over}>Over the limit</span>}
        </div>
        {(soft ?? hard) && (
          <>
            <div className={styles.meter}>
              <i style={{ width: share(dollars(billed)) }} />
              {softDollars !== null && (
                <b className={styles.softMark} style={{ left: share(softDollars) }} />
              )}
              {hardDollars !== null && (
                <b className={styles.hardMark} style={{ left: share(hardDollars) }} />
              )}
            </div>
            <div className={styles.meterKey}>
              <span>$0</span>
              <span>
                {softDollars !== null ? `Email at $${String(softDollars)}` : ''}
                {hardDollars !== null ? ` · stop at $${String(hardDollars)}` : ''}
              </span>
            </div>
          </>
        )}
      </section>
      <SettingGroup
        title="Usage limits"
        description={`These are ${name}’s own limits, so they work even when Hoista is closed.`}
        note={
          customer
            ? undefined
            : `${name} didn’t say which customer this is, so the limits can only be changed in ${name}.`
        }
      >
        <SelectRow
          label="Email me when the bill reaches"
          hint={`${name} sends the email. Nothing stops.`}
          value={softDollars}
          options={SOFT_LIMITS}
          onChange={(next) => {
            setLimits(next, next === null ? null : hardDollars);
          }}
        />
        <SelectRow
          label="Stop everything at"
          hint={`${name} shuts down every service in this account until the next period. Set an email amount first.`}
          value={hardDollars}
          options={hardOptions}
          onChange={(next) => {
            setLimits(softDollars, next);
          }}
        />
      </SettingGroup>
      <ErrorMessage error={run.error} />
      <p className={shared.small}>
        {name} bills the account, not each project, so Hoista shows no cost per project.
      </p>
    </>
  );
}
