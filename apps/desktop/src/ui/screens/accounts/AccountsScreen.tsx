/**
 * Accounts: what Hoista can reach. A banner for any token that stopped
 * working, a table of connected accounts (who it is, its status, which projects
 * use it, when it last worked), then the providers not connected yet: the ones your projects
 * need first, then the rest grouped by kind. Connect and Manage open as
 * dialogs over this screen.
 */
import { entryOf, providerEntry, serviceName } from '@shared/providers';
import { Link, Outlet, useNavigate } from 'react-router-dom';
import type { Account } from '@shared/api';
import { useAccounts, useCatalogue, useConnectGuide, fetchAccount } from '@ui/api/accounts';
import { openExternal } from '@ui/api/system';
import { RowMenu } from '@ui/components/RowMenu';
import { showToast } from '@ui/lib/toast';
import { ButtonLink } from '@ui/components/Button';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { Dot } from '@ui/components/Dot';
import { ProviderTiles } from '@ui/components/ProviderTiles';
import { Page } from '@ui/components/Page';
import { QueryState } from '@ui/components/QueryState';
import { SkeletonTableBody } from '@ui/components/Skeleton';
import { PROVIDER_GROUPS, PROVIDERS, type Provider } from '@ui/lib/providers';
import { listOf, plural, timeAgo } from '@ui/lib/format';
import { ServiceLogo } from './ServiceLogo';
import styles from './AccountsScreen.module.css';

/** How many project names a card lists before "and N more". */
const SHOWN_PROJECTS = 3;

export function AccountsScreen() {
  const accounts = useAccounts();

  return (
    <Page>
      <h1 className="visually-hidden">Accounts</h1>
      <QueryState query={accounts} skeleton={<AccountsSkeleton />}>
        {(data) => <AccountsBody accounts={data.items} />}
      </QueryState>
      <Outlet />
    </Page>
  );
}

/** The connected table as it will be, while the accounts load. */
function AccountsSkeleton() {
  return (
    <section className={styles.section}>
      <h2 className={styles.sectionTitle}>Connected</h2>
      <div className={styles.tableFrame}>
        <table className={styles.table}>
          <thead>
            <tr>
              <th scope="col">Account</th>
              <th scope="col">Status</th>
              <th scope="col">Used by</th>
              <th scope="col">Last checked</th>
              <th scope="col">
                <span className="visually-hidden">Actions</span>
              </th>
            </tr>
          </thead>
          <SkeletonTableBody count={4} widths={[150, 90, 180, 70, null]} />
        </table>
      </div>
    </section>
  );
}

function AccountsBody({ accounts }: { accounts: Account[] }) {
  const navigate = useNavigate();
  const catalogue = useCatalogue();
  const broken = accounts.filter((account) => account.status !== 'ok');
  const connected = new Set<string>(accounts.map((account) => account.service));
  const others = PROVIDERS.filter((provider) => !connected.has(provider.id));

  /** Providers a project of yours needs, with the reason in place of the usual description. */
  const reasons = new Map(
    (catalogue.data?.items ?? [])
      .filter((entry) => entry.recommended_because)
      .map((entry) => [entry.id, entry.recommended_because ?? ''] as const),
  );
  const recommended: Provider[] = others
    .filter((provider) => reasons.has(provider.id))
    .map((provider) => ({ ...provider, for: reasons.get(provider.id) ?? provider.for }));
  const rest = others.filter((provider) => !reasons.has(provider.id));
  const connect = (provider: string) => {
    void navigate(`/accounts/connect/${provider}`);
  };

  return (
    <>
      {broken.map((account) => (
        <ProblemBanner key={account.id} account={account} />
      ))}

      {accounts.length > 0 && (
        <section className={styles.section}>
          <h2 className={styles.sectionTitle}>Connected</h2>
          <div className={styles.tableFrame}>
            <table className={styles.table}>
              <thead>
                <tr>
                  <th scope="col">Account</th>
                  <th scope="col">Status</th>
                  <th scope="col">Used by</th>
                  <th scope="col">Last checked</th>
                  <th scope="col">
                    <span className="visually-hidden">Actions</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {accounts.map((account) => (
                  <AccountRow key={account.id} account={account} />
                ))}
              </tbody>
            </table>
          </div>
        </section>
      )}

      {/* Two columns only with recommendations beside the rest; otherwise the full width. */}
      <div className={recommended.length > 0 ? styles.split : styles.tiles}>
        {recommended.length > 0 && (
          <section className={styles.section}>
            <h2 className={styles.sectionTitle}>Recommended for your projects</h2>
            <ProviderTiles
              providers={recommended}
              label="Recommended for your projects"
              onPick={connect}
            />
          </section>
        )}
        <div className={styles.section}>
          {PROVIDER_GROUPS.map((group) => {
            const providers = rest.filter((provider) => provider.group === group.id);
            if (providers.length === 0) return null;
            return (
              <section key={group.id} className={styles.section}>
                <h2 className={styles.sectionTitle}>{group.title}</h2>
                <ProviderTiles providers={providers} label={group.title} onPick={connect} />
              </section>
            );
          })}
        </div>
      </div>

      <p className={styles.more}>More providers are on the way, one at a time.</p>
    </>
  );
}

/** Access that stopped working, said plainly, with the one thing to do about it. */
function ProblemBanner({ account }: { account: Account }) {
  const name = serviceName(account.service);
  const projects = account.used_by;
  const oneClick = Boolean(useConnectGuide(account.service).data?.sign_in);
  const fix = oneClick ? 'reconnected' : 'replaced';

  return (
    <div className={styles.banner} role="alert">
      <span className={styles.bannerLogo}>
        <ServiceIcon id={account.service} size={16} />
      </span>
      <div className={styles.bannerText}>
        <strong>
          {oneClick ? `${name} needs you to reconnect` : `${name} token stopped working`}
        </strong>
        <span>
          {projects.length > 0
            ? `${listOf(projects)} can’t deploy or report until it’s ${fix}.`
            : `Nothing on it can deploy or report until it’s ${fix}.`}
        </span>
      </div>
      <ButtonLink to={`/accounts/${account.id}`} variant="accent" large>
        {oneClick ? 'Reconnect' : 'Replace token'}
      </ButtonLink>
    </div>
  );
}

/** A connected account's actions: manage, open the provider, check, reconnect, disconnect. */
function AccountMenu({ account }: { account: Account }) {
  const navigate = useNavigate();
  const name = serviceName(account.service);
  const dashboard = providerEntry(account.service)?.dashboard?.(account.identity) ?? null;
  const signIn = useConnectGuide(account.service).data?.sign_in ?? null;
  return (
    <RowMenu
      label={`More for ${name}`}
      items={[
        {
          label: 'Manage',
          onSelect: () => {
            void navigate(`/accounts/${account.id}`);
          },
        },
        ...(dashboard
          ? [
              {
                label: `Open ${name} ↗`,
                onSelect: () => {
                  void openExternal(dashboard);
                },
              },
            ]
          : []),
        {
          label: 'Check connection',
          onSelect: () => {
            showToast(`Checking ${name}`);
            fetchAccount(account.id)
              .then((result) => {
                showToast(result.problem ? `${name}: ${result.problem.message}` : `${name} works`);
              })
              .catch(() => {
                showToast(`Couldn’t reach ${name}. Try again in a moment.`, { tone: 'error' });
              });
          },
        },
        {
          label: signIn ? 'Reconnect' : 'Replace token',
          onSelect: () => {
            void navigate(`/accounts/${account.id}?section=token`);
          },
        },
        {
          label: 'Disconnect',
          danger: true,
          onSelect: () => {
            void navigate(`/accounts/${account.id}?remove=1`);
          },
        },
      ]}
    />
  );
}

function AccountRow({ account }: { account: Account }) {
  const name = serviceName(account.service);
  const isBroken = account.status !== 'ok';
  const shown = account.used_by.slice(0, SHOWN_PROJECTS);
  const hidden = account.used_by.length - shown.length;
  const noun = entryOf(account.service).catalogue.category === 'code' ? 'repo' : 'project';

  return (
    <tr>
      <td>
        <span className={styles.accountCell}>
          <ServiceLogo id={account.service} name={name} />
          <span className={styles.cardName}>
            <Link to={`/accounts/${account.id}`} className={styles.name}>
              {name}
            </Link>
            <span className={styles.identity}>{account.label}</span>
          </span>
        </span>
      </td>
      <td>
        <span className={isBroken ? styles.bad : styles.good}>
          <Dot tone={isBroken ? 'bad' : 'good'} small /> {isBroken ? 'Token invalid' : 'Connected'}
        </span>
      </td>
      <td>
        {account.used_by.length === 0 ? (
          <span className={styles.muted}>{`No ${noun}s yet`}</span>
        ) : (
          <span className={styles.projects}>
            {shown.map((project) => (
              <span key={project} className={styles.project}>
                {project}
              </span>
            ))}
            {hidden > 0 && <span className={styles.muted}>{`+${plural(hidden, noun)}`}</span>}
          </span>
        )}
      </td>
      <td className={styles.muted}>
        {account.last_worked_at ? timeAgo(account.last_worked_at) : 'never'}
      </td>
      <td className={styles.actionCell}>
        <AccountMenu account={account} />
      </td>
    </tr>
  );
}
