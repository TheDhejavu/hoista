/**
 * Settings · Instances: every Hoista this computer can show, with whether
 * each answers and whether this computer is signed in to it.
 * Each row's ⋮ has the rest: show it, sign in, rename, push this
 * computer's projects there, open its page, sign out, remove it here.
 */
import { useState } from 'react';
import { Laptop, Server } from 'lucide-react';
import type { RemoteItem } from '@shared/api';
import {
  useForgetRemote,
  useRemotes,
  useRemotesContents,
  useRenameRemote,
  useShowRemote,
  useSignOutRemote,
} from '@ui/api/settings';
import { openExternal } from '@ui/api/system';
import { Button } from '@ui/components/Button';
import { Dialog, DialogText } from '@ui/components/Dialog';
import { Dot } from '@ui/components/Dot';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { TextField } from '@ui/components/Field';
import { RowMenu } from '@ui/components/RowMenu';
import { SettingGroup } from '@ui/components/SettingRow';
import { Skeleton } from '@ui/components/Skeleton';
import { cx } from '@ui/lib/cx';
import { AddRemoteDialog } from './AddRemoteDialog';
import { MoveToServerDialog } from './MoveToServerDialog';
import { pushChoice } from './PushOffer';
import styles from './RemotesList.module.css';

/** How a remote is doing, in a word and a colour. */

export function remoteState(remote: RemoteItem): { label: string; tone: 'good' | 'warn' | 'off' } {
  if (remote.reachable === false) return { label: 'Not answering', tone: 'off' };
  if (!remote.signed_in) return { label: 'Signed out', tone: 'warn' };
  return { label: 'Signed in', tone: 'good' };
}

function hostOf(url: string): string {
  return url.replace(/^https?:\/\//, '');
}

export function RemotesList() {
  const remotes = useRemotes();
  const show = useShowRemote();
  const signOut = useSignOutRemote();
  const [adding, setAdding] = useState(false);
  const [signingIn, setSigningIn] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<RemoteItem | null>(null);
  const [pushing, setPushing] = useState<RemoteItem | null>(null);
  const [removing, setRemoving] = useState<RemoteItem | null>(null);
  const data = remotes.data;
  const showing = data?.showing ?? 'this_computer';
  // A push is offered only into an empty remote: once it holds anything, the
  // entry goes, so nothing can be merged into it.
  const checkable = (data?.items ?? []).filter(
    (remote) => remote.signed_in && remote.reachable !== false && remote.id !== showing,
  );
  const contents = useRemotesContents(checkable.map((remote) => remote.id));
  const empty = new Set(
    checkable.flatMap((remote, index) => {
      const read = contents[index]?.data;
      return read && pushChoice(read) === 'offer' ? [remote.id] : [];
    }),
  );

  return (
    <SettingGroup>
      <div className={styles.head}>
        <div className={styles.headText}>
          <h3 className={styles.title}>Your instances</h3>
          <p className={styles.description}>
            This Mac and the Hoistas you run on servers of yours. Switch between them here or from
            the menu at the bottom left.
          </p>
        </div>
        <Button
          onClick={() => {
            setAdding(true);
          }}
        >
          Add a remote
        </Button>
      </div>
      {remotes.isLoading ? (
        <div className={styles.list} aria-busy="true">
          <Skeleton height={44} />
          <Skeleton height={44} />
        </div>
      ) : (
        <ul
          className={cx(styles.list, (data?.items.length ?? 0) > 0 && styles.withMenus)}
          aria-label="Your remotes"
        >
          <li className={styles.row}>
            <span className={styles.icon}>
              <Laptop size={16} aria-hidden="true" />
            </span>
            <span className={styles.text}>
              <span className={styles.name}>This Mac</span>
              <span className={styles.sub}>Local instance</span>
            </span>
            <span className={styles.state}>
              {showing === 'this_computer' ? (
                <span className={styles.showing}>Current</span>
              ) : (
                <Button
                  variant="ghost"
                  busy={show.isPending && show.variables === 'this_computer'}
                  busyLabel="Switching"
                  onClick={() => {
                    show.mutate('this_computer');
                  }}
                >
                  Switch
                </Button>
              )}
            </span>
          </li>
          {data?.items.map((remote) => {
            const state = remoteState(remote);
            const shown = showing === remote.id;
            return (
              <li key={remote.id} className={styles.row}>
                <span className={styles.icon}>
                  <Server size={16} aria-hidden="true" />
                </span>
                <span className={styles.text}>
                  <span className={styles.name}>{remote.name}</span>
                  <span className={cx(styles.sub, 'mono')}>{hostOf(remote.url)}</span>
                </span>
                <span className={styles.state}>
                  {shown ? (
                    <span className={styles.showing}>Current</span>
                  ) : (
                    <span className={styles.status}>
                      <Dot tone={state.tone} small />
                      {state.label}
                    </span>
                  )}
                </span>
                <RowMenu
                  label={`More for ${remote.name}`}
                  items={[
                    ...(remote.signed_in
                      ? shown
                        ? []
                        : [
                            {
                              label: 'Switch to it',
                              onSelect: () => {
                                show.mutate(remote.id);
                              },
                            },
                          ]
                      : [
                          {
                            label: 'Sign in',
                            onSelect: () => {
                              setSigningIn(remote.id);
                            },
                          },
                        ]),
                    {
                      label: 'Rename',
                      onSelect: () => {
                        setRenaming(remote);
                      },
                    },
                    ...(empty.has(remote.id)
                      ? [
                          {
                            label: 'Push this Mac’s projects to it',
                            onSelect: () => {
                              setPushing(remote);
                            },
                          },
                        ]
                      : []),
                    {
                      label: 'Open its page',
                      onSelect: () => {
                        void openExternal(remote.url);
                      },
                    },
                    ...(remote.signed_in
                      ? [
                          {
                            label: 'Sign out',
                            onSelect: () => {
                              signOut.mutate(remote.id);
                            },
                          },
                        ]
                      : []),
                    {
                      label: 'Remove from this Mac',
                      danger: true,
                      onSelect: () => {
                        setRemoving(remote);
                      },
                    },
                  ]}
                />
              </li>
            );
          })}
          {data?.items.length === 0 && (
            <li className={styles.emptyRow}>
              No remotes yet. Add one to reach your Hoista when this Mac is off, from chat apps and
              cloud agents too.
            </li>
          )}
        </ul>
      )}
      <ErrorMessage error={signOut.error ?? remotes.error} />
      {adding && (
        <AddRemoteDialog
          onClose={() => {
            setAdding(false);
          }}
        />
      )}
      {signingIn && (
        <AddRemoteDialog
          signInTo={signingIn}
          onClose={() => {
            setSigningIn(null);
          }}
        />
      )}
      {renaming && (
        <RenameRemoteDialog
          remote={renaming}
          onClose={() => {
            setRenaming(null);
          }}
        />
      )}
      {pushing && (
        <MoveToServerDialog
          remoteId={pushing.id}
          provider={pushing.name}
          onClose={() => {
            setPushing(null);
          }}
        />
      )}
      {removing && (
        <ForgetRemoteDialog
          remote={removing}
          onClose={() => {
            setRemoving(null);
          }}
        />
      )}
    </SettingGroup>
  );
}

function RenameRemoteDialog({ remote, onClose }: { remote: RemoteItem; onClose: () => void }) {
  const [name, setName] = useState(remote.name);
  const rename = useRenameRemote();
  return (
    <Dialog
      title="Rename remote"
      eyebrow={hostOf(remote.url)}
      onClose={onClose}
      actions={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            busy={rename.isPending}
            busyLabel="Saving"
            disabled={!name.trim() || name.trim() === remote.name}
            onClick={() => {
              rename.mutate({ id: remote.id, name }, { onSuccess: onClose });
            }}
          >
            Save
          </Button>
        </>
      }
    >
      <TextField
        label="Name"
        value={name}
        hint="Only on this Mac. The remote itself isn’t changed."
        onChange={setName}
      />
      <ErrorMessage error={rename.error} />
    </Dialog>
  );
}

function ForgetRemoteDialog({ remote, onClose }: { remote: RemoteItem; onClose: () => void }) {
  const forget = useForgetRemote();
  return (
    <Dialog
      title={`Remove ${remote.name} from this Mac?`}
      eyebrow={hostOf(remote.url)}
      onClose={onClose}
      actions={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="danger"
            busy={forget.isPending}
            busyLabel="Removing"
            onClick={() => {
              forget.mutate(remote.id, { onSuccess: onClose });
            }}
          >
            Remove
          </Button>
        </>
      }
    >
      <DialogText>
        <p>
          This Mac forgets it and its key. {remote.name} keeps running where it is, with its
          projects and agents. Add it again any time with its address.
        </p>
        <p>To stop the key working there too, remove this Mac on its page.</p>
      </DialogText>
      <ErrorMessage error={forget.error} />
    </Dialog>
  );
}
