import { providerGuide } from '@ui/lib/providerGuides';
import { serviceName } from '@shared/providers';
import { useState } from 'react';
import type { Account, TokenCheck } from '@shared/api';
import { useConnectGuide, useReplaceToken } from '@ui/api/accounts';
import { Button } from '@ui/components/Button';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { TokenField } from '@ui/components/TokenField';
import { FixedValue, SettingGroup, SettingRow } from '@ui/components/SettingRow';
import { OrUseToken, SignInOption } from '@ui/components/SignInOption';
import { timeAgo } from '@ui/lib/format';
import { ProviderGuideDialog } from '@ui/screens/guides/ProviderGuideScreen';
import styles from './TokenSection.module.css';
import shared from './account.module.css';
import { Permissions } from './Permissions';

export function TokenSection({
  account,
  replacing,
  onReplacing: setReplacing,
  onReplaced,
}: {
  account: Account;
  replacing: boolean;
  onReplacing: (replacing: boolean) => void;
  onReplaced: (checks: TokenCheck[]) => void;
}) {
  const signIn = useConnectGuide(account.service).data?.sign_in ?? null;

  return (
    <>
      <SettingGroup title="The token">
        <SettingRow
          label="Where it’s kept"
          hint="Hoista never shows it again or sends it anywhere but the provider. Connected with a 1Password reference, the token stays in 1Password and Hoista keeps only the reference."
        >
          <FixedValue>
            {account.kept_in === '1password'
              ? '1Password · Hoista keeps only the reference'
              : account.token_hint
                ? `Your keychain · ends ${account.token_hint}`
                : 'Your keychain'}
          </FixedValue>
        </SettingRow>
        <SettingRow label="Added">
          <FixedValue>{timeAgo(account.connected_at)}</FixedValue>
        </SettingRow>
        {!replacing && (
          <SettingRow label={signIn ? 'Sign in again or use a new token' : 'Use a new token'}>
            <Button
              onClick={() => {
                setReplacing(true);
              }}
            >
              {signIn ? 'Reconnect' : 'Replace token'}
            </Button>
          </SettingRow>
        )}
        {replacing && (
          <ReplaceTokenPanel
            account={account}
            onReplaced={onReplaced}
            onCancel={() => {
              setReplacing(false);
            }}
          />
        )}
      </SettingGroup>
      <Permissions account={account} />
    </>
  );
}

/**
 * Signing in again, or pasting a new token, for one account, right where the
 * person asked for it (the Token section, or Settings → Vault).
 */
export function ReplaceTokenPanel({
  account,
  onReplaced,
  onCancel,
}: {
  account: Account;
  onReplaced: (checks: TokenCheck[]) => void;
  onCancel: () => void;
}) {
  const name = serviceName(account.service);
  const replace = useReplaceToken(account.id);
  const signIn = useConnectGuide(account.service).data?.sign_in ?? null;
  const [token, setToken] = useState('');
  // The provider's guide to making a token, over this panel.
  const [guiding, setGuiding] = useState(false);

  function submit(): void {
    replace.mutate(token, {
      onSuccess: (result) => {
        onReplaced(result.checks);
      },
      // The token is only needed for this one request.
      onSettled: () => {
        setToken('');
      },
    });
  }

  return (
    <section
      className={styles.replaceInside}
      ref={(element) => {
        element?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
      }}
    >
      {account.problem && <p className={styles.problemText}>{account.problem.message}</p>}
      {signIn && (
        <>
          <SignInOption service={account.service} details={signIn} again onConnected={onReplaced} />
          <OrUseToken label="or paste a new token" />
        </>
      )}
      <TokenField
        label={`New ${name} token`}
        value={token}
        placeholder="Paste the new token"
        onChange={setToken}
      />
      {providerGuide(account.service) && (
        <button
          type="button"
          className={shared.inline}
          onClick={() => {
            setGuiding(true);
          }}
        >
          Show me how to make one
        </button>
      )}
      {guiding && (
        <ProviderGuideDialog
          provider={account.service}
          step="token"
          onClose={() => {
            setGuiding(false);
          }}
        />
      )}
      <ErrorMessage error={replace.error} />
      <div className={styles.buttons}>
        <Button
          onClick={() => {
            setToken('');
            onCancel();
          }}
        >
          Cancel
        </Button>
        <Button variant="accent" disabled={!token.trim() || replace.isPending} onClick={submit}>
          {replace.isPending ? 'Checking' : 'Verify & replace'}
        </Button>
      </div>
    </section>
  );
}
