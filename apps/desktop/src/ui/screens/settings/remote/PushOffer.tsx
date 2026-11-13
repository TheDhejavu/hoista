/**
 * Bringing this Mac's projects to a remote, once, into an empty remote only
 * (the way Postman offers to move local work into your account when you sign
 * in). Nothing is merged: a remote that already holds projects or accounts
 * is used as it is, or this Mac is.
 */
import type { RemoteContents, ServerMove } from '@shared/api';
import { QueryState } from '@ui/components/QueryState';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { useRemoteContents } from '@ui/api/settings';
import { plural } from '@ui/lib/format';
import styles from './AddRemoteDialog.module.css';

/** What to offer for a remote, from what it and this Mac hold. */
type PushChoice = 'offer' | 'taken' | 'nothing';

export function pushChoice(contents: RemoteContents): PushChoice {
  const { remote, here } = contents;
  if (remote.projects > 0 || remote.accounts > 0) return 'taken';
  if (here.projects === 0 && here.accounts === 0) return 'nothing';
  return 'offer';
}

/** What a push carries, in a line. */
function carried(here: RemoteContents['here']): string {
  return `${plural(here.projects, 'project')} and ${plural(here.accounts, 'provider account')}`;
}

/** The read of both sides, then the offer, the reason there's none, or what moved. */
export function PushPanel({
  remoteId,
  name,
  done,
  error,
  heading = true,
}: {
  remoteId: string;
  name: string;
  /** Off when the window's own title already says it. */
  heading?: boolean;
  done: ServerMove | undefined;
  error: Error | null;
}) {
  const contents = useRemoteContents(remoteId);
  if (done) {
    return (
      <div className={styles.offer} role="status">
        <span className={styles.guideTitle}>Push complete</span>
        <p className={styles.hint}>
          {plural(done.projects, 'project')} and {plural(done.sign_ins, 'provider account')} are on{' '}
          {name}. Agents on this Mac are paused. Connect your agents to {name}; each one signs in
          once.
        </p>
        {done.left_watching.length > 0 && (
          <p className={styles.hint}>
            Set to watching on {name} until you switch them to managed there:{' '}
            <strong>{done.left_watching.join(', ')}</strong>.
          </p>
        )}
      </div>
    );
  }
  return (
    <QueryState query={contents} skeleton={<></>}>
      {(data) => {
        const choice = pushChoice(data);
        if (choice === 'nothing') return null;
        if (choice === 'taken') {
          return (
            <div className={styles.offer}>
              <span className={styles.guideTitle}>{name} isn’t empty</span>
              <p className={styles.hint}>
                It has {plural(data.remote.projects, 'project')} and{' '}
                {plural(data.remote.accounts, 'provider account')}. Use it as it is, or keep using
                this Mac. Projects aren’t merged.
              </p>
            </div>
          );
        }
        return (
          <div className={styles.offer}>
            {heading && (
              <span className={styles.guideTitle}>Push this Mac’s projects to {name}?</span>
            )}
            <p className={styles.hint}>
              {name} has no projects or accounts yet. This pushes {carried(data.here)} to it:
            </p>
            <ul className={styles.offerList}>
              <li>Encrypted with a one-time key. Each project keeps its mode.</li>
              <li>Agents on this Mac are paused afterwards; they connect to {name} instead.</li>
              <li>Nothing on this Mac is deleted. You can push only once, into an empty remote.</li>
            </ul>
            <ErrorMessage error={error} />
          </div>
        );
      }}
    </QueryState>
  );
}
