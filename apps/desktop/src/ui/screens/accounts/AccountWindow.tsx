/**
 * One connected account, as a window like Settings: its sections on the
 * left, the chosen one on the right. Every provider has Overview and Token;
 * providers that report a bill add Bill and usage limits, and providers whose API
 * has them add Alerts and Projects. Everything shown is read from the provider;
 * changes go to the provider's own settings (Railway's usage limits are
 * Railway's, and work with Hoista closed).
 */
import { X } from 'lucide-react';
import { useSearchParams } from 'react-router-dom';
import { RowMenu } from '@ui/components/RowMenu';
import { entryOf, serviceName, PROVIDER_IDS, providerEntry } from '@shared/providers';
import { useState } from 'react';
import type { Account, ProviderTarget, TokenCheck } from '@shared/api';
import { useBills } from '@ui/api/accounts';
import { useProviderOperations } from '@ui/api/providerOperations';
import { openExternal } from '@ui/api/system';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { SettingGroupSkeleton } from '@ui/components/SettingRow';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { Skeleton } from '@ui/components/Skeleton';
import { cx } from '@ui/lib/cx';
import { showToast } from '@ui/lib/toast';
import { formatCost } from '@ui/lib/format';
import settings from '../settings/SettingsDialog.module.css';
import styles from './AccountWindow.module.css';
import shared from './account.module.css';
import { SECOND_MS } from '@shared/time';
import { projectsWord } from './accountWords';
import { OverviewSection } from './OverviewSection';
import { BillSection } from './BillSection';
import { AlertsSection } from './AlertsSection';
import { ProjectsSection, RepositoriesSection } from './ProjectsSection';
import { TokenSection } from './TokenSection';

/** A check shows as busy at least this long, so a quick answer still reads as a check. */
const MIN_CHECK_MS = 0.6 * SECOND_MS;

type Section = 'overview' | 'bill' | 'alerts' | 'projects' | 'repositories' | 'token';

const TITLES: Record<Section, string> = {
  overview: 'Overview',
  bill: 'Bill and usage limits',
  alerts: 'Alerts',
  projects: 'Projects',
  repositories: 'Repositories',
  token: 'Token',
};

/**
 * The account's window as it will be, while the account loads: its menu and
 * a section, with placeholders for what's in them.
 */
export function AccountWindowSkeleton({ onClose }: { onClose: () => void }) {
  return (
    <Dialog title="Account" onClose={onClose} bare>
      <div className={settings.window}>
        <nav aria-label="Account" className={settings.nav}>
          <div className={styles.navHead}>
            <Skeleton width={36} height={36} />
            <div className={styles.skeletonName}>
              <Skeleton width={110} height={14} />
              <Skeleton width={70} height={10} />
            </div>
          </div>
          <div className={settings.group}>
            {[90, 120, 60].map((width) => (
              <div key={width} className={settings.link}>
                <Skeleton width={width} height={12} />
              </div>
            ))}
          </div>
        </nav>
        <div className={settings.main}>
          <header className={settings.head}>
            <Skeleton width={180} height={22} />
            <div className={styles.headActions} />
            <button type="button" className={settings.close} aria-label="Close" onClick={onClose}>
              <X size={15} aria-hidden="true" />
            </button>
          </header>
          <div className={settings.content}>
            <SettingGroupSkeleton cards={2} rows={3} />
          </div>
        </div>
      </div>
    </Dialog>
  );
}

export function AccountWindow({
  account,
  onCheck,
  onReplaced,
  onRemove,
  onClose,
}: {
  account: Account;
  onCheck: () => Promise<Account | undefined>;
  onReplaced: (checks: TokenCheck[]) => void;
  onRemove: () => void;
  onClose: () => void;
}) {
  const name = serviceName(account.service);
  const target: ProviderTarget = { kind: 'account', id: account.id };
  const operations = useProviderOperations(target).data ?? [];
  const has = (operation: string) => operations.some((item) => item.name === operation);
  const bill = useBills().data?.find((item) => item.account_id === account.id) ?? null;
  // A broken token opens on the Token section, where it gets fixed.
  // A row's ⋮ can open a section directly (`?section=token` to reconnect).
  const [search] = useSearchParams();
  const asked = search.get('section');
  const [section, setSection] = useState<Section>(
    asked === 'token' || account.problem ? 'token' : 'overview',
  );
  const [replacing, setReplacing] = useState(account.problem !== null || asked === 'token');
  const [running, setRunning] = useState(false);
  const [checkedAt, setCheckedAt] = useState<string | null>(null);
  const dashboard = providerEntry(account.service)?.dashboard?.(account.identity) ?? null;

  // Asks the provider again and says how it went, even when it's quick.
  async function check(): Promise<void> {
    setRunning(true);
    const [result] = await Promise.all([
      onCheck().catch(() => undefined),
      new Promise((resolve) => setTimeout(resolve, MIN_CHECK_MS)),
    ]);
    setRunning(false);
    setCheckedAt(new Date().toISOString());
    if (!result) showToast(`Couldn’t reach ${name}. Try again in a moment.`, { tone: 'error' });
    else if (result.problem) showToast(`${name}: ${result.problem.message}`);
    else showToast(`${name} works`);
  }

  const sections: { id: Section; label: string; count?: string }[] = [
    { id: 'overview', label: 'Overview' },
    ...(bill
      ? [
          {
            id: 'bill' as const,
            label: 'Usage and limits',
            count: formatCost(bill.bill.billed),
          },
        ]
      : []),
    ...(has('notificationRules') ? [{ id: 'alerts' as const, label: 'Alerts' }] : []),
    // Every provider Hoista finds things on has this, in the provider's own word.
    ...(PROVIDER_IDS.includes(account.service)
      ? [{ id: 'projects' as const, label: projectsWord(account.service) }]
      : []),
    // Where code lives (GitHub): the repositories its token reaches.
    ...(entryOf(account.service).catalogue.category === 'code'
      ? [{ id: 'repositories' as const, label: 'Repositories' }]
      : []),
    { id: 'token', label: 'Token' },
  ];

  return (
    <Dialog title={name} onClose={onClose} bare>
      <div className={settings.window}>
        <nav aria-label={`${name} account`} className={settings.nav}>
          <div className={styles.navHead}>
            <span className={styles.logo}>
              <ServiceIcon id={account.service} size={18} />
            </span>
            <div>
              <div className={styles.navName} id="dialog-title">
                {name}
              </div>
              <div className={styles.navStatus}>
                <span className={cx(shared.dot, account.problem ? shared.bad : shared.good)} />
                {account.problem ? 'Token invalid' : 'Connected'}
              </div>
            </div>
          </div>
          <div className={settings.group}>
            <div className={settings.groupTitle}>{account.label}</div>
            {sections.map((item) => (
              <button
                key={item.id}
                type="button"
                aria-current={item.id === section ? 'page' : undefined}
                className={cx(
                  settings.link,
                  styles.navLink,
                  item.id === section && settings.active,
                )}
                onClick={() => {
                  setSection(item.id);
                }}
              >
                <span>{item.label}</span>
                {item.count && <span className={styles.count}>{item.count}</span>}
              </button>
            ))}
          </div>
        </nav>
        <div className={settings.main}>
          <header className={settings.head}>
            <h1>{section === 'projects' ? projectsWord(account.service) : TITLES[section]}</h1>
            <span className={settings.saves}>{subtitle(section, name)}</span>
            <div className={styles.headActions}>
              {dashboard && (
                <Button
                  onClick={() => {
                    void openExternal(dashboard);
                  }}
                >
                  Open {name} ↗
                </Button>
              )}
              <RowMenu
                label={`More for ${name}`}
                items={[
                  {
                    label: running ? 'Checking' : 'Check connection',
                    disabled: running,
                    onSelect: () => {
                      void check();
                    },
                  },
                  {
                    label: 'Replace token',
                    onSelect: () => {
                      setSection('token');
                      setReplacing(true);
                    },
                  },
                  { label: 'Disconnect', danger: true, onSelect: onRemove },
                ]}
              />
            </div>
            <button type="button" className={settings.close} aria-label="Close" onClick={onClose}>
              <X size={15} aria-hidden="true" />
            </button>
          </header>
          <div className={settings.content}>
            {section === 'overview' && (
              <OverviewSection
                account={account}
                target={target}
                workspace={has('workspace')}
                facts={(providerEntry(account.service)?.accountFacts ?? []).filter((facts) =>
                  has(facts.op),
                )}
                bill={bill}
                running={running}
                checkedAt={checkedAt}
                onCheck={() => void check()}
                onFix={() => {
                  setSection('token');
                  setReplacing(true);
                }}
                onToken={() => {
                  setSection('token');
                }}
              />
            )}
            {section === 'bill' && bill && <BillSection account={account} bill={bill} />}
            {section === 'alerts' && (
              <AlertsSection target={target} name={name} service={account.service} />
            )}
            {section === 'projects' && <ProjectsSection account={account} />}
            {section === 'repositories' && <RepositoriesSection account={account} />}
            {section === 'token' && (
              <TokenSection
                account={account}
                replacing={replacing}
                onReplacing={setReplacing}
                onReplaced={onReplaced}
              />
            )}
          </div>
        </div>
      </div>
    </Dialog>
  );
}

function subtitle(section: Section, name: string): string {
  switch (section) {
    case 'overview':
      return `Read from ${name}`;
    case 'bill':
      return `For the whole account, from ${name}`;
    case 'alerts':
      return `${name}’s own alert rules`;
    case 'projects':
      return `Everything in this account at ${name}`;
    case 'repositories':
      return `What this token can read on ${name}`;
    case 'token':
      return 'Kept in your keychain';
  }
}
