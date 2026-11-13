/**
 * Confirms removing an account's token from the vault. Projects keep running at
 * the provider; Hoista just can't manage them until a token is added again.
 */
import type { Account } from '@shared/api';
import { serviceName } from '@shared/providers';
import { useDisconnectAccount } from '@ui/api/accounts';
import { Button } from '@ui/components/Button';
import { Dialog, DialogText } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { listOf } from '@ui/lib/format';

export function RemoveTokenDialog({ account, onClose }: { account: Account; onClose: () => void }) {
  const remove = useDisconnectAccount(account.id);
  const name = serviceName(account.service);
  const projects = account.used_by.length ? listOf(account.used_by) : 'Your projects';

  return (
    <Dialog
      title={`Remove the ${name} token?`}
      eyebrow="Settings · Vault"
      onClose={onClose}
      actions={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="danger"
            disabled={remove.isPending}
            onClick={() => {
              remove.mutate(undefined, { onSuccess: onClose });
            }}
          >
            Remove token
          </Button>
        </>
      }
    >
      <DialogText>
        <p>Hoista deletes the token from your keychain. Nothing changes on {name}.</p>
        <ul>
          <li>
            {projects} keep running on {name}.
          </li>
          <li>
            You and your agents can't deploy, check or roll them back until you add a token again.
          </li>
          <li>To fully cut access, also delete the token in {name}'s own settings.</li>
        </ul>
        <ErrorMessage error={remove.error} />
      </DialogText>
    </Dialog>
  );
}
