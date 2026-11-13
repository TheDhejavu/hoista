/**
 * Step 1 of connecting a source, the way providers' own "Connect Git repository"
 * screens do it: GitHub or a local folder, then for GitHub an account, a
 * search and a list to pick from. Picking only selects; the review step
 * says what changes before anything does.
 */
import { Check, Folder, Globe, Lock, Search } from 'lucide-react';
import { useMemo, useState, type ReactNode } from 'react';
import { useAccounts } from '@ui/api/accounts';
import { getBridge } from '@ui/api/bridge';
import { useBranches, useRepositories } from '@ui/api/repositories';
import { Button, ButtonLink } from '@ui/components/Button';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { PillSelect } from '@ui/components/Select';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { openExternal } from '@ui/api/system';
import { cx } from '@ui/lib/cx';
import { timeAgo } from '@ui/lib/format';
import styles from './SourcePicker.module.css';

type Kind = 'github' | 'folder';

/** What the person picked: a repository (with its account, unless the provider suggested it) or a folder. */
export type SourceChoice =
  | {
      kind: 'github';
      repository: string;
      branch: string;
      accountId: string | null;
      /** Set when the provider said which commit it deploys. */
      commit: string | null;
      private: boolean | null;
    }
  | { kind: 'folder'; path: string };

export function SourcePicker({
  provider,
  offer,
  choice,
  onChoose,
}: {
  provider: string;
  /** The repository and commit the provider deploys, when it said. */
  offer: { repository: string; commit: string } | null;
  choice: SourceChoice | null;
  onChoose: (choice: SourceChoice | null) => void;
}) {
  const [kind, setKind] = useState<Kind | null>(choice?.kind ?? null);
  const offered =
    offer !== null && choice?.kind === 'github' && choice.repository === offer.repository;

  return (
    <div className={styles.picker}>
      {offer && (
        <button
          type="button"
          className={cx(styles.offer, offered && styles.offerOn)}
          aria-pressed={offered}
          onClick={() => {
            setKind(null);
            onChoose({
              kind: 'github',
              repository: offer.repository,
              branch: '',
              accountId: null,
              commit: offer.commit,
              private: null,
            });
          }}
        >
          <span className={styles.tile}>
            <ServiceIcon id="github" size={16} />
          </span>
          <span className={styles.offerText}>
            <span className={styles.offerLabel}>Suggested · {provider} deploys this now</span>
            <span className={styles.repoName}>{offer.repository}</span>
          </span>
          <span className={styles.radio} aria-hidden="true" />
        </button>
      )}
      <div className={styles.kinds} role="radiogroup" aria-label="Source">
        <KindCard
          selected={kind === 'github'}
          icon={<ServiceIcon id="github" size={18} />}
          title="GitHub repository"
          text={`${provider} builds from its commits.`}
          onSelect={() => {
            setKind('github');
            if (choice?.kind !== 'github' || offered) onChoose(null);
          }}
        />
        <KindCard
          selected={kind === 'folder'}
          icon={<Folder size={18} aria-hidden="true" />}
          title="Local folder"
          text="A folder on this computer that you and your agents work in."
          onSelect={() => {
            setKind('folder');
            if (choice?.kind !== 'folder') onChoose(null);
          }}
        />
      </div>
      {kind === 'github' && (
        <RepositoryBrowser
          chosen={choice?.kind === 'github' && !offered ? choice : null}
          onChoose={onChoose}
        />
      )}
      {kind === 'folder' && (
        <div className={styles.folder}>
          <span className={styles.tile}>
            <Folder size={16} aria-hidden="true" />
          </span>
          <span className={styles.folderText}>
            {choice?.kind === 'folder' ? (
              <span className="mono">{choice.path}</span>
            ) : (
              'Choose the project’s root folder, where its package or build file is.'
            )}
          </span>
          <Button
            variant={choice?.kind === 'folder' ? 'secondary' : 'accent'}
            onClick={() => {
              void getBridge()
                .chooseFolder()
                .then((path) => {
                  if (path) onChoose({ kind: 'folder', path });
                });
            }}
          >
            {choice?.kind === 'folder' ? 'Choose another' : 'Choose folder'}
          </Button>
        </div>
      )}
    </div>
  );
}

function KindCard({
  selected,
  icon,
  title,
  text,
  onSelect,
}: {
  selected: boolean;
  icon: ReactNode;
  title: string;
  text: string;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      className={cx(styles.kind, selected && styles.kindOn)}
      onClick={onSelect}
    >
      <span className={styles.kindIcon}>{icon}</span>
      <span className={styles.kindText}>
        <span className={styles.kindTitle}>{title}</span>
        <span className={styles.kindHint}>{text}</span>
      </span>
      <span className={styles.radio} aria-hidden="true" />
    </button>
  );
}

/** A GitHub account's repositories: search, then pick the one this project runs. */
function RepositoryBrowser({
  chosen,
  onChoose,
}: {
  chosen: Extract<SourceChoice, { kind: 'github' }> | null;
  onChoose: (choice: SourceChoice) => void;
}) {
  const allAccounts = useAccounts();
  const accounts = (allAccounts.data?.items ?? []).filter(
    (account) => account.service === 'github',
  );
  const [chosenAccount, setChosenAccount] = useState<string | null>(null);
  // One account needs no choosing; several start at None.
  const account =
    accounts.length === 1
      ? (accounts[0] ?? null)
      : (accounts.find((item) => item.id === chosenAccount) ?? null);
  const repos = useRepositories(account?.id ?? null);
  const [query, setQuery] = useState('');

  const shown = useMemo(() => {
    const words = query.trim().toLowerCase();
    return (repos.data ?? []).filter(
      (repo) =>
        !words ||
        repo.full_name.toLowerCase().includes(words) ||
        (repo.description ?? '').toLowerCase().includes(words),
    );
  }, [repos.data, query]);

  if (allAccounts.isPending) {
    return (
      <p className={styles.status} role="status">
        <span className={styles.spinner} aria-hidden="true" />
        Looking for your GitHub accounts
      </p>
    );
  }
  if (accounts.length === 0) {
    return (
      <div className={styles.folder}>
        <span className={styles.tile}>
          <ServiceIcon id="github" size={16} />
        </span>
        <span className={styles.folderText}>
          Connect a GitHub account to see its repositories here.
        </span>
        <ButtonLink to="/accounts/connect/github" variant="accent">
          Connect GitHub
        </ButtonLink>
      </div>
    );
  }

  return (
    <div className={styles.browser}>
      <div className={styles.tools}>
        {accounts.length > 1 ? (
          <PillSelect
            label="Account"
            value={account?.id ?? null}
            options={[
              { value: null as string | null, label: 'None' },
              ...accounts.map((item) => ({ value: item.id, label: item.label })),
            ]}
            onChange={(id) => {
              setChosenAccount(id);
              setQuery('');
            }}
          />
        ) : (
          <span className={styles.owner}>
            <ServiceIcon id="github" size={14} />
            {account?.label}
          </span>
        )}
        <label className={styles.search}>
          <Search size={14} aria-hidden="true" />
          <input
            type="search"
            placeholder="Search repositories"
            value={query}
            disabled={!account}
            onChange={(event) => {
              setQuery(event.target.value);
            }}
          />
        </label>
      </div>
      <div className={styles.list} role="radiogroup" aria-label="Repositories">
        {!account ? (
          <p className={styles.status}>Choose an account to see its repositories.</p>
        ) : repos.isPending ? (
          <p className={styles.status} role="status">
            <span className={styles.spinner} aria-hidden="true" />
            Reading repositories on GitHub
          </p>
        ) : repos.error ? null : shown.length === 0 ? (
          <p className={styles.status}>
            {query ? `No repositories match “${query}”.` : 'This token reaches no repositories.'}
          </p>
        ) : (
          shown.map((repo) => {
            const on = chosen?.repository === repo.full_name;
            return (
              <button
                key={repo.full_name}
                type="button"
                role="radio"
                aria-checked={on}
                className={cx(styles.repo, on && styles.repoOn)}
                onClick={() => {
                  onChoose({
                    kind: 'github',
                    repository: repo.full_name,
                    branch: repo.default_branch,
                    accountId: account.id,
                    commit: null,
                    private: repo.private,
                  });
                }}
              >
                <span className={styles.visibility} title={repo.private ? 'Private' : 'Public'}>
                  {repo.private ? (
                    <Lock size={14} aria-label="Private" />
                  ) : (
                    <Globe size={14} aria-label="Public" />
                  )}
                </span>
                <span className={styles.repoText}>
                  <span className={styles.repoName}>{repo.full_name}</span>
                  <span className={styles.repoMeta}>
                    {[
                      repo.default_branch,
                      repo.pushed_at ? `pushed ${timeAgo(repo.pushed_at)}` : null,
                      repo.description,
                    ]
                      .filter(Boolean)
                      .join(' · ')}
                  </span>
                </span>
                {on && <Check size={16} aria-hidden="true" className={styles.check} />}
              </button>
            );
          })
        )}
      </div>
      {chosen && account && (
        <BranchRow
          accountId={account.id}
          chosen={chosen}
          defaultBranch={
            (repos.data ?? []).find((repo) => repo.full_name === chosen.repository)
              ?.default_branch ?? chosen.branch
          }
          onChoose={onChoose}
        />
      )}
      <ErrorMessage error={repos.error} />
    </div>
  );
}

/** The branch to deploy, from the repository's own branches; its default to start. */
function BranchRow({
  accountId,
  chosen,
  defaultBranch,
  onChoose,
}: {
  accountId: string;
  chosen: Extract<SourceChoice, { kind: 'github' }>;
  defaultBranch: string;
  onChoose: (choice: SourceChoice) => void;
}) {
  const branches = useBranches(accountId, chosen.repository);
  const names = branches.data ?? [defaultBranch];
  // The default first, then the rest as GitHub lists them.
  const ordered = [defaultBranch, ...names.filter((name) => name !== defaultBranch)];
  return (
    <div className={styles.branchRow}>
      <span className={styles.branchLabel}>
        Branch for <b>{chosen.repository}</b>
      </span>
      {branches.isPending ? (
        <span className={styles.repoMeta} role="status">
          <span className={styles.spinner} aria-hidden="true" />
          Reading branches
        </span>
      ) : (
        <PillSelect
          label="Branch"
          value={chosen.branch}
          options={ordered.map((name) => ({
            value: name,
            label: name,
            ...(name === defaultBranch ? { note: 'default' } : {}),
          }))}
          onChange={(branch) => {
            onChoose({ ...chosen, branch });
          }}
        />
      )}
      <ErrorMessage error={branches.error} />
    </div>
  );
}

/** The connected source as one row: what it is, where it opens, and its ⋮. */
export function CurrentSource({
  kind,
  name,
  url,
  meta,
  menu,
}: {
  kind: Kind;
  name: string;
  url: string | null;
  meta: ReactNode;
  menu: ReactNode;
}) {
  return (
    <div className={styles.current}>
      <span className={styles.tile}>
        {kind === 'github' ? (
          <ServiceIcon id="github" size={16} />
        ) : (
          <Folder size={16} aria-hidden="true" />
        )}
      </span>
      <span className={styles.currentText}>
        {url ? (
          <button
            type="button"
            className={cx(styles.currentName, styles.nameButton)}
            onClick={() => {
              void openExternal(url);
            }}
          >
            {name} ↗
          </button>
        ) : (
          <span className={cx(styles.currentName, kind === 'folder' && 'mono')}>{name}</span>
        )}
        <span className={styles.currentMeta}>{meta}</span>
      </span>
      {menu}
    </div>
  );
}
