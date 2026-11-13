import { ChevronRight } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { useProjects } from '@ui/api/projects';
import { ALL_ENVIRONMENTS } from '@ui/api/environments';
import { at, written } from '@ui/screens/project/resources/ProviderFacts';
import type { ProviderFacts } from '@shared/providers/types';
import { entryOf, serviceName } from '@shared/providers';
import type { Account, AccountBill, ProviderTarget } from '@shared/api';
import { useProviderRead } from '@ui/api/providerOperations';
import { Button } from '@ui/components/Button';
import { FixedValue, SettingGroup, SettingRow } from '@ui/components/SettingRow';
import { SkeletonRows } from '@ui/components/Skeleton';
import { cx } from '@ui/lib/cx';
import { regionName } from '@ui/lib/regions';
import { formatCost, plural, shortDate, timeAgo } from '@ui/lib/format';
import styles from './OverviewSection.module.css';
import shared from './account.module.css';
import { projectsWord, providerWords, words } from './accountWords';

interface Workspace {
  name?: string;
  plan?: string | null;
  preferredRegion?: string | null;
  projectCount?: number | null;
}

export function OverviewSection({
  account,
  target,
  workspace: hasWorkspace,
  facts,
  bill,
  running,
  checkedAt,
  onCheck,
  onFix,
  onToken,
}: {
  account: Account;
  target: ProviderTarget;
  workspace: boolean;
  facts: ProviderFacts[];
  bill: AccountBill | null;
  running: boolean;
  checkedAt: string | null;
  onCheck: () => void;
  onFix: () => void;
  onToken: () => void;
}) {
  const name = serviceName(account.service);
  const workspace = useProviderRead<Workspace>(hasWorkspace ? target : null, 'workspace').data;

  return (
    <>
      {account.problem && (
        <div className={styles.problem}>
          <div>
            <strong>
              Your {plural(account.used_by.length, 'project')} on {name} keep running.
            </strong>{' '}
            {account.problem.message} Hoista and your agents can’t deploy or check them until the
            token works again.
          </div>
          <Button onClick={onFix}>Fix the token</Button>
        </div>
      )}
      {(workspace ?? bill) && (
        <SettingGroup title="Workspace">
          {workspace?.name && (
            <SettingRow label="Name">
              <FixedValue>{workspace.name}</FixedValue>
            </SettingRow>
          )}
          {workspace?.plan && (
            <SettingRow label="Plan">
              <FixedValue>{words(workspace.plan)}</FixedValue>
            </SettingRow>
          )}
          {workspace?.preferredRegion && (
            <SettingRow
              label="Default region"
              hint={`New services start here unless you choose another · ${workspace.preferredRegion}`}
            >
              <FixedValue>{regionName(workspace.preferredRegion)}</FixedValue>
            </SettingRow>
          )}
          {typeof workspace?.projectCount === 'number' && (
            <SettingRow label={projectsWord(account.service)}>
              <FixedValue>{String(workspace.projectCount)}</FixedValue>
            </SettingRow>
          )}
          {bill && (
            <SettingRow
              label="Used so far"
              hint={
                bill.bill.period_start && bill.bill.period_end
                  ? `${shortDate(bill.bill.period_start)} – ${shortDate(bill.bill.period_end)}`
                  : undefined
              }
            >
              <FixedValue>{formatCost(bill.bill.billed)} this period</FixedValue>
            </SettingRow>
          )}
        </SettingGroup>
      )}
      {facts.map((item) => (
        <AccountFacts key={item.op} facts={item} target={target} />
      ))}
      <UsedBy account={account} />
      <SettingGroup title="Connection">
        <SettingRow label="Connected as" hint={`The ${name} login or team the token belongs to`}>
          <span className={styles.plainValue}>{account.identity}</span>
        </SettingRow>
        <SettingRow
          label="Status"
          hint={
            checkedAt
              ? `Checked ${timeAgo(checkedAt)}`
              : account.last_worked_at
                ? `The token worked ${timeAgo(account.last_worked_at)}`
                : 'Not used yet'
          }
        >
          <span className={styles.statusValue}>
            <span className={cx(shared.dot, account.problem ? shared.bad : shared.good)} />
            {account.problem ? 'Token invalid' : 'Connected'}
          </span>
          <Button busy={running} busyLabel="Checking" onClick={onCheck}>
            Check now
          </Button>
        </SettingRow>
        <SettingRow label="Token" hint={`Added ${timeAgo(account.connected_at)}`}>
          <FixedValue>
            {account.kept_in === '1password'
              ? '1Password'
              : account.token_hint
                ? `Keychain · ends ${account.token_hint}`
                : 'Keychain'}
          </FixedValue>
          <Button variant="ghost" onClick={onToken}>
            Manage
          </Button>
        </SettingRow>
      </SettingGroup>
    </>
  );
}

/** One of the provider's account-wide reads, as rows; left out when the provider doesn't answer. */
function AccountFacts({ facts, target }: { facts: ProviderFacts; target: ProviderTarget }) {
  const read = useProviderRead(target, facts.op, facts.args ?? {});
  if (read.isPending) {
    return (
      <SettingGroup title={facts.title}>
        <SkeletonRows count={Math.min(3, facts.rows?.length ?? 2)} />
      </SettingGroup>
    );
  }
  const rows = (facts.rows ?? []).flatMap((fact) => {
    const text = written(at(read.data, fact.path), fact.as);
    return text === null ? [] : [{ fact, text }];
  });
  if (rows.length === 0) return null;
  return (
    <SettingGroup title={facts.title}>
      {rows.map(({ fact, text }) => (
        <SettingRow key={fact.label} label={fact.label}>
          <FixedValue>
            {fact.as === 'code' ? <span className="mono">{text}</span> : providerWords(text)}
          </FixedValue>
        </SettingRow>
      ))}
    </SettingGroup>
  );
}

/** The projects in Hoista that use this account, each opening its page; GitHub lists repositories. */
function UsedBy({ account }: { account: Account }) {
  const navigate = useNavigate();
  const projects = useProjects({ environment: ALL_ENVIRONMENTS }).data?.items ?? [];
  const code = entryOf(account.service).catalogue.category === 'code';
  return (
    <SettingGroup
      title={code ? 'Linked repositories' : 'Projects using it'}
      description={
        account.used_by.length === 0
          ? undefined
          : `${plural(account.used_by.length, code ? 'repository' : 'project')} in Hoista`
      }
    >
      {account.used_by.length === 0 && (
        <SettingRow
          label="None yet"
          hint={
            code
              ? 'Link a repository in a project’s Settings.'
              : 'Bring a project in from Projects.'
          }
        />
      )}
      {account.used_by.map((used) => {
        const project = code ? undefined : projects.find((item) => item.name === used);
        return project ? (
          <button
            key={used}
            type="button"
            className={styles.projectRow}
            onClick={() => {
              void navigate(`/projects/${project.id}`);
            }}
          >
            <span>{used}</span>
            <ChevronRight size={15} aria-hidden="true" />
          </button>
        ) : (
          <SettingRow key={used} label={used} />
        );
      })}
    </SettingGroup>
  );
}
