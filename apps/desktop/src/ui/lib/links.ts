/**
 * Addresses of things Hoista shows that live somewhere else: a repository
 * and a commit on GitHub. A value that isn't one is no link.
 */

const REPO = /^[A-Za-z0-9_.-]{1,100}\/[A-Za-z0-9_.-]{1,100}$/;
const COMMIT = /^[0-9a-f]{7,64}$/i;

/** A repository's page, from `owner/name`. */
export function repoUrl(repo: string | null | undefined): string | null {
  const name = repo?.replace(/\.git$/, '').replace(/^https:\/\/github\.com\//, '');
  return name && REPO.test(name) ? `https://github.com/${name}` : null;
}

/** A commit's page on GitHub. */
export function commitUrl(
  repo: string | null | undefined,
  commit: string | null | undefined,
): string | null {
  const base = repoUrl(repo);
  return base && commit && COMMIT.test(commit) ? `${base}/commit/${commit}` : null;
}

/** `owner/name@sha`, as the project's linked source reads, split in two. */
export function splitSource(ref: string): { repo: string; commit: string | null } {
  const at = ref.lastIndexOf('@');
  return at === -1
    ? { repo: ref, commit: null }
    : { repo: ref.slice(0, at), commit: ref.slice(at + 1) || null };
}
