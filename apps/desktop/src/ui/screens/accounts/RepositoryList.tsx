/**
 * A code account's repositories as GitHub's own list shows them: search,
 * a Type and Sort dropdown, then one row each (name, visibility,
 * description, default branch, last push) with its ⋮. Shown a page at a
 * time, with Show more, so a long list stays quick.
 */
import { Globe, Lock, Search } from 'lucide-react';
import { useMemo, useState } from 'react';
import type { ProviderRepository } from '@engine-api';
import { useRepositories } from '@ui/api/repositories';
import { openExternal } from '@ui/api/system';
import { Button } from '@ui/components/Button';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { PillSelect, type Option } from '@ui/components/Select';
import { SkeletonRows } from '@ui/components/Skeleton';
import { timeAgo } from '@ui/lib/format';
import { RepositoryMenu } from './RepositoryMenu';
import styles from './RepositoryList.module.css';

const PAGE = 20;

type Visibility = 'public' | 'private' | null;
type Sort = 'pushed' | 'name';

const TYPES: Option<Visibility>[] = [
  { value: null, label: 'All' },
  { value: 'public', label: 'Public' },
  { value: 'private', label: 'Private' },
];

const SORTS: Option<Sort>[] = [
  { value: 'pushed', label: 'Last pushed' },
  { value: 'name', label: 'Name' },
];

export function RepositoryList({ accountId }: { accountId: string }) {
  const repos = useRepositories(accountId);
  const [query, setQuery] = useState('');
  const [type, setType] = useState<Visibility>(null);
  const [sort, setSort] = useState<Sort>('pushed');
  const [shown, setShown] = useState(PAGE);

  const matching = useMemo(() => {
    const words = query.trim().toLowerCase();
    const found = (repos.data ?? []).filter(
      (repo) =>
        (type === null || (type === 'private') === repo.private) &&
        (!words ||
          repo.full_name.toLowerCase().includes(words) ||
          (repo.description ?? '').toLowerCase().includes(words)),
    );
    return sort === 'name'
      ? [...found].sort((a, b) => a.full_name.localeCompare(b.full_name))
      : [...found].sort((a, b) => (b.pushed_at ?? '').localeCompare(a.pushed_at ?? ''));
  }, [repos.data, query, type, sort]);

  const total = repos.data?.length ?? 0;
  const page = matching.slice(0, shown);

  return (
    <section className={styles.card} aria-label="Repositories">
      <div className={styles.tools}>
        <label className={styles.search}>
          <Search size={14} aria-hidden="true" />
          <input
            type="search"
            placeholder="Find a repository"
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setShown(PAGE);
            }}
          />
        </label>
        <PillSelect
          label="Type"
          value={type}
          options={TYPES}
          onChange={(next) => {
            setType(next);
            setShown(PAGE);
          }}
        />
        <PillSelect label="Sort" value={sort} options={SORTS} onChange={setSort} />
      </div>

      {repos.isPending ? (
        <div className={styles.loading} role="status" aria-label="Reading repositories on GitHub">
          <SkeletonRows count={5} />
        </div>
      ) : repos.error ? (
        <div className={styles.empty}>
          <ErrorMessage error={repos.error} />
        </div>
      ) : total === 0 ? (
        <p className={styles.empty}>
          This token reaches no repositories. Give it access to the ones your projects are built
          from, then check it again.
        </p>
      ) : matching.length === 0 ? (
        <p className={styles.empty}>No repositories match.</p>
      ) : (
        <ul className={styles.list}>
          {page.map((repo) => (
            <RepositoryRow key={repo.full_name} repo={repo} />
          ))}
        </ul>
      )}

      {matching.length > 0 && (
        <div className={styles.foot}>
          <span>
            {matching.length === total
              ? `${String(Math.min(shown, total))} of ${String(total)}`
              : `${String(Math.min(shown, matching.length))} of ${String(matching.length)} matching · ${String(total)} in all`}
          </span>
          {matching.length > shown && (
            <Button
              onClick={() => {
                setShown(shown + PAGE);
              }}
            >
              Show more
            </Button>
          )}
        </div>
      )}
    </section>
  );
}

function RepositoryRow({ repo }: { repo: ProviderRepository }) {
  const [owner, name] = repo.full_name.split('/');
  return (
    <li className={styles.row}>
      <span className={styles.icon} title={repo.private ? 'Private' : 'Public'}>
        {repo.private ? (
          <Lock size={15} aria-hidden="true" />
        ) : (
          <Globe size={15} aria-hidden="true" />
        )}
      </span>
      <span className={styles.text}>
        <span className={styles.titleLine}>
          <button
            type="button"
            className={styles.name}
            onClick={() => {
              void openExternal(repo.url);
            }}
          >
            <span className={styles.owner}>{owner}/</span>
            {name}
          </button>
          <span className={styles.badge}>{repo.private ? 'Private' : 'Public'}</span>
        </span>
        {repo.description && <span className={styles.description}>{repo.description}</span>}
        <span className={styles.meta}>
          <span className="mono">{repo.default_branch}</span>
          {repo.pushed_at && <span>Updated {timeAgo(repo.pushed_at)}</span>}
        </span>
      </span>
      <RepositoryMenu repo={repo} />
    </li>
  );
}
