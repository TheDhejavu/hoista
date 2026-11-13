/**
 * Where a resource's code comes from, under its name, the way providers show
 * it: the GitHub logo, the repository, the branch and the commit, each
 * opening on GitHub; or a folder icon and the folder. Nothing when the project's
 * code isn't linked.
 */
import { Folder, Globe } from 'lucide-react';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { openExternal } from '@ui/api/system';
import { commitUrl, repoUrl } from '@ui/lib/links';
import styles from './ResourceScreen.module.css';

export function SourceLine({
  repo,
  branch,
  commit,
  folder,
}: {
  repo?: string | null;
  branch?: string | null;
  commit?: string | null;
  folder?: string | null;
}) {
  const repoLink = repoUrl(repo);
  if (repo && repoLink) {
    const commitLink = commitUrl(repo, commit);
    const link = (url: string, text: string, mono = false) => (
      <button
        type="button"
        className={mono ? `${styles.sourceLink} mono` : styles.sourceLink}
        onClick={() => {
          void openExternal(url);
        }}
      >
        {text}
      </button>
    );
    return (
      <div className={styles.source}>
        <ServiceIcon id="github" size={14} />
        {link(repoLink, repo)}
        {branch && (
          <>
            <span className={styles.sourceDot}>·</span>
            {link(`${repoLink}/tree/${encodeURIComponent(branch)}`, branch, true)}
          </>
        )}
        {commit && commitLink && (
          <>
            <span className={styles.sourceDot}>·</span>
            {link(commitLink, commit.slice(0, 7), true)}
          </>
        )}
      </div>
    );
  }
  if (folder) {
    return (
      <div className={styles.source}>
        <Folder size={14} aria-hidden="true" />
        <span className="mono">{folder}</span>
      </div>
    );
  }
  return null;
}

/** The address a resource answers at, under its name; opens it. */
export function AddressLine({ url }: { url?: string | null }) {
  if (!url) return null;
  const provider = url.replace(/^https?:\/\//, '').replace(/\/.*$/, '');
  return (
    <div className={styles.source}>
      <Globe size={14} aria-hidden="true" />
      <button
        type="button"
        className={`${styles.sourceLink} mono`}
        onClick={() => {
          void openExternal(url);
        }}
      >
        {provider} ↗
      </button>
    </div>
  );
}
