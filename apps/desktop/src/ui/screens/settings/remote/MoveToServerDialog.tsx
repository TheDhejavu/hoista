/**
 * Pushing this computer's Hoista to a remote, from the remote's ⋮ menu:
 * projects, accounts and their sign-ins go over sealed, each project keeps
 * its mode, and agents here are paused so only the remote takes changes.
 * Only into an empty remote, once; nothing is merged.
 */
import { usePushToRemote, useRemoteContents, useShowRemote } from '@ui/api/settings';
import { Button } from '@ui/components/Button';
import { Dialog, DialogText } from '@ui/components/Dialog';
import { PushPanel, pushChoice } from './PushOffer';

export function MoveToServerDialog({
  remoteId,
  provider,
  onClose,
}: {
  remoteId: string;
  /** The remote's name. */
  provider: string;
  onClose: () => void;
}) {
  const move = usePushToRemote();
  const show = useShowRemote();
  const contents = useRemoteContents(remoteId);
  const choice = contents.data ? pushChoice(contents.data) : null;
  const done = move.data;

  const actions = done ? (
    <>
      <Button variant="ghost" disabled={show.isPending} onClick={onClose}>
        Keep using this Mac
      </Button>
      <Button
        variant="primary"
        busy={show.isPending}
        busyLabel="Switching"
        onClick={() => {
          show.mutate(remoteId, { onSuccess: onClose });
        }}
      >
        Switch to {provider}
      </Button>
    </>
  ) : choice === 'offer' ? (
    <>
      <Button variant="ghost" disabled={move.isPending} onClick={onClose}>
        Cancel
      </Button>
      <Button
        variant="primary"
        busy={move.isPending}
        busyLabel="Pushing"
        onClick={() => {
          move.mutate(remoteId);
        }}
      >
        Push to {provider}
      </Button>
    </>
  ) : (
    <Button variant="primary" disabled={contents.isPending} onClick={onClose}>
      Close
    </Button>
  );

  return (
    <Dialog
      title={done ? `Pushed to ${provider}` : `Push this Mac’s projects to ${provider}?`}
      eyebrow="Instances"
      onClose={move.isPending ? () => undefined : onClose}
      actions={actions}
    >
      <DialogText>
        {choice === 'nothing' && !done ? (
          <p>This Mac has no projects or provider accounts to push.</p>
        ) : (
          <PushPanel
            remoteId={remoteId}
            name={provider}
            done={done}
            error={move.error}
            heading={false}
          />
        )}
      </DialogText>
    </Dialog>
  );
}
