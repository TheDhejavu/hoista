/**
 * Connecting one provider account, as hooks any screen can lay out its own
 * way: the Connect dialog, and the provider cards in first-run setup.
 *
 * - `useTokenConnect`: paste a token. Hoista asks which accounts it reaches,
 *   asks you to pick one when there are several (or type its ID when the token
 *   can't list them), then checks and saves it.
 * - `useBrowserSignIn`: approve Hoista on the provider's own page, then wait
 *   for the account to arrive.
 */
import { useEffect, useRef, useState } from 'react';
import type { AccountChoice, ConnectGuide, TokenCheck } from '@shared/api';
import { accountIdFields } from '@shared/catalogue';
import {
  useAccounts,
  useConnectAccount,
  useLookupAccounts,
  useSignInAccount,
} from '@ui/api/accounts';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { TextField } from '@ui/components/Field';
import { SelectField } from '@ui/components/Select';
import { POLL_MS } from '@ui/api/polling';

/** Paste a token and connect with it. `onConnected` gets the engine's checks. */
export function useTokenConnect(guide: ConnectGuide, onConnected: (checks: TokenCheck[]) => void) {
  const lookup = useLookupAccounts();
  const connect = useConnectAccount();
  const [token, setToken] = useState('');
  // Set when the token reaches several accounts; the person picks one.
  const [choices, setChoices] = useState<AccountChoice[] | null>(null);
  const [chosen, setChosen] = useState<string>('');
  // Set when the token cannot list its accounts; the person types the ID.
  const [needsId, setNeedsId] = useState(false);
  const [accountId, setAccountId] = useState('');
  const idField = accountIdFields[guide.service] ?? {
    label: 'Account ID',
    hint: 'The account’s ID at the service.',
  };

  function save(choice: AccountChoice | null): void {
    connect.mutate(
      choice
        ? { service: guide.service, token, external_id: choice.external_id, label: choice.name }
        : { service: guide.service, token },
      {
        onSuccess: (result) => {
          onConnected(result.checks);
        },
        // The token is only needed for this one request.
        onSettled: () => {
          setToken('');
          setChoices(null);
          setNeedsId(false);
          setAccountId('');
        },
      },
    );
  }

  function submit(): void {
    if (needsId) {
      const id = accountId.trim();
      // The ID looks like a secret and means nothing on its own; label it plainly.
      save({
        external_id: id,
        name: `${guide.name} ${idField.label.replace(/ ID$/i, '').toLowerCase()}`,
      });
      return;
    }
    if (choices) {
      save(choices.find((choice) => choice.external_id === chosen) ?? null);
      return;
    }
    lookup.mutate(
      { service: guide.service, token },
      {
        onSuccess: (found) => {
          const [only] = found;
          if (found.length === 1 && only) {
            save(only);
            return;
          }
          if (found.length > 1) {
            setChoices(found);
            setChosen(found[0]?.external_id ?? '');
            return;
          }
          setNeedsId(true);
        },
        // Some tokens cannot list accounts (a Railway workspace token can't).
        // Keep the token in the field and ask for the account instead.
        onError: () => {
          setNeedsId(true);
        },
      },
    );
  }

  // A different token starts over: what was asked of the last one (its
  // account ID, which of its accounts) and its errors no longer apply.
  function changeToken(value: string): void {
    setToken(value);
    if (value.trim() === token.trim()) return;
    setChoices(null);
    setChosen('');
    setNeedsId(false);
    setAccountId('');
    lookup.reset();
    connect.reset();
  }

  const isBusy = lookup.isPending || connect.isPending;
  return {
    token,
    setToken: changeToken,
    submit,
    isBusy,
    /** Ready to send: a token, and the account ID when one is asked for. */
    canSubmit: token.trim() !== '' && (!needsId || accountId.trim() !== '') && !isBusy,
    /** What the connect button says. */
    actionLabel: isBusy ? 'Checking' : choices ? 'Connect this account' : 'Verify & connect',
    choices,
    chosen,
    setChosen,
    needsId,
    accountId,
    setAccountId,
    idField,
    lookupError: lookup.error,
    connectError: connect.error,
  };
}

export type TokenConnect = ReturnType<typeof useTokenConnect>;

/**
 * What a token connection may still need under its field: the account ID
 * when the token can't list its accounts, which account when it reaches
 * several, and what went wrong.
 */
export function TokenAccountFields({ connect }: { connect: TokenConnect }) {
  return (
    <>
      {connect.needsId && (
        <>
          <p>
            This token can’t list its accounts, so enter the {connect.idField.label.toLowerCase()}.
          </p>
          <TextField
            label={connect.idField.label}
            value={connect.accountId}
            onChange={connect.setAccountId}
            hint={connect.idField.hint}
          />
        </>
      )}
      {connect.choices && (
        <SelectField
          label="Which account?"
          value={connect.chosen}
          options={connect.choices.map((choice) => ({
            value: choice.external_id,
            label: choice.name,
          }))}
          onChange={connect.setChosen}
        />
      )}
      {!connect.needsId && <ErrorMessage error={connect.lookupError} />}
      <ErrorMessage error={connect.connectError} />
    </>
  );
}

/**
 * Connect on the provider's own page. `open` opens it in the browser; the
 * result arrives when you click Allow there. Once it's open, a new account
 * for the provider also counts, whichever sign-in saved it (an earlier page,
 * or one opened again), so `onConnected` runs once either way.
 */
export function useBrowserSignIn(guide: ConnectGuide, onConnected: (checks: TokenCheck[]) => void) {
  const signIn = useSignInAccount();
  // Only a sign-in started here counts: an account connected some other way
  // (a token pasted beside it) reports for itself.
  const opened = useRef(false);
  const finished = useRef(false);
  const [done, setDone] = useState(false);
  // Accounts for this provider that were already there, so a new one is noticed.
  const accounts = useAccounts(signIn.isPending ? POLL_MS.watching : undefined);
  const before = useRef<Set<string> | null>(null);
  const items = accounts.data?.items;

  function connected(checks: TokenCheck[]): void {
    if (finished.current) return;
    finished.current = true;
    setDone(true);
    onConnected(checks);
  }

  useEffect(() => {
    if (!items || finished.current) return;
    const mine = items.filter((item) => item.service === guide.service);
    if (before.current === null) {
      before.current = new Set(mine.map((item) => item.id));
      return;
    }
    const known = before.current;
    if (opened.current && mine.some((item) => !known.has(item.id))) {
      connected([{ label: `Signed in on ${guide.name}`, ok: true, note: null }]);
    }
  });

  function open(): void {
    opened.current = true;
    signIn.mutate(
      { service: guide.service },
      {
        onSuccess: (result) => {
          connected(result.checks);
        },
      },
    );
  }

  return { open, isPending: signIn.isPending, error: signIn.error, done };
}
