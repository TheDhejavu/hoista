/**
 * Manage one connected account at `/accounts/:accountId`, in a window like
 * Settings (see AccountWindow): what it is, its bill and limits, its alerts
 * and projects, and its token. Removing it never deletes anything at the
 * provider.
 */
import { useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import type { Account, TokenCheck } from '@shared/api';
import { serviceName } from '@shared/providers';
import { useAccount, useDisconnectAccount } from '@ui/api/accounts';
import { Button } from '@ui/components/Button';
import { showToast } from '@ui/lib/toast';
import { Dialog, DialogText } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { QueryState } from '@ui/components/QueryState';
import { TokenChecks } from '@ui/components/TokenChecks';
import { useRequiredParam } from '@ui/lib/useRequiredParam';
import { AccountWindow, AccountWindowSkeleton } from './AccountWindow';

export function ManageAccountDialog() {
  const accountId = useRequiredParam('accountId');
  const navigate = useNavigate();
  const account = useAccount(accountId);

  function close(): void {
    void navigate('/accounts');
  }

  return (
    <QueryState query={account} skeleton={<AccountWindowSkeleton onClose={close} />}>
      {(data) => (
        <ManageAccount
          account={data}
          onCheck={async () => (await account.refetch()).data}
          onClose={close}
        />
      )}
    </QueryState>
  );
}

/** Which dialog is showing: the account, the disconnect question, or the new token's checks. */
type View =
  { name: 'account' } | { name: 'disconnect' } | { name: 'replaced'; checks: TokenCheck[] };

function ManageAccount({
  account,
  onCheck,
  onClose,
}: {
  account: Account;
  onCheck: () => Promise<Account | undefined>;
  onClose: () => void;
}) {
  // A row's ⋮ can open the window on its remove question (`?remove=1`).
  const [search] = useSearchParams();
  const [view, setView] = useState<View>(
    search.get('remove') ? { name: 'disconnect' } : { name: 'account' },
  );

  if (view.name === 'replaced') {
    return <TokenReplacedDialog account={account} checks={view.checks} onClose={onClose} />;
  }

  // The remove question opens on top of the account window, which stays.
  return (
    <>
      <AccountWindow
        account={account}
        onCheck={onCheck}
        onReplaced={(checks) => {
          // All good: say so in a toast and get out of the way. The dialog
          // stays only when a check failed, so there's something to act on.
          if (checks.every((check) => check.ok)) {
            showToast(`${serviceName(account.service)} is connected again`);
            onClose();
            return;
          }
          setView({ name: 'replaced', checks });
        }}
        onRemove={() => {
          setView({ name: 'disconnect' });
        }}
        onClose={onClose}
      />
      {view.name === 'disconnect' && (
        <ConfirmDisconnectDialog
          account={account}
          onKeep={() => {
            setView({ name: 'account' });
          }}
          onClose={onClose}
        />
      )}
    </>
  );
}

function TokenReplacedDialog({
  account,
  checks,
  onClose,
}: {
  account: Account;
  checks: TokenCheck[];
  onClose: () => void;
}) {
  return (
    <Dialog
      title={`${serviceName(account.service)} needs a look`}
      onClose={onClose}
      actions={
        <Button variant="primary" onClick={onClose}>
          Done
        </Button>
      }
    >
      <TokenChecks checks={checks} />
    </Dialog>
  );
}

function ConfirmDisconnectDialog({
  account,
  onKeep,
  onClose,
}: {
  account: Account;
  onKeep: () => void;
  onClose: () => void;
}) {
  const name = serviceName(account.service);
  const disconnect = useDisconnectAccount(account.id);

  return (
    <Dialog
      title={`Disconnect ${name}?`}
      onClose={onKeep}
      actions={
        <>
          <Button variant="ghost" onClick={onKeep}>
            Keep it
          </Button>
          <Button
            variant="danger"
            disabled={disconnect.isPending}
            onClick={() => {
              disconnect.mutate(undefined, { onSuccess: onClose });
            }}
          >
            Disconnect
          </Button>
        </>
      }
    >
      <DialogText>
        <p>
          Disconnecting doesn’t delete anything. They keep running in your {name} account; Hoista
          just stops managing them and removes the token from your keychain.
        </p>
        <ErrorMessage error={disconnect.error} />
      </DialogText>
    </Dialog>
  );
}
