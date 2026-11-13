/**
 * Settings · Vault: where Hoista keeps provider tokens, as the vault in use says
 * (this computer's keychain, or the remote's sealed file), and the tokens it
 * holds there.
 */
import { useState } from 'react';
import type { VaultInfo } from '@engine-api';
import type { Account } from '@shared/api';
import { serviceName } from '@shared/providers';
import { useAccounts, useConnectGuide } from '@ui/api/accounts';
import { useVaultInfo } from '@ui/api/settings';
import { Button } from '@ui/components/Button';
import { QueryState } from '@ui/components/QueryState';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import {
  FixedValue,
  SettingGroup,
  SettingRow,
  SettingGroupSkeleton,
} from '@ui/components/SettingRow';
import { listOf, timeAgo } from '@ui/lib/format';
import { showToast } from '@ui/lib/toast';
import { ReplaceTokenPanel } from '@ui/screens/accounts/TokenSection';
import { RemoveTokenDialog } from './RemoveTokenDialog';
import { RowMenu } from '@ui/components/RowMenu';
import { SectionHeader } from './SectionHeader';
import styles from './VaultSettings.module.css';

export function VaultSettings() {
  const accounts = useAccounts();
  const vault = useVaultInfo();

  return (
    <>
      <SectionHeader title="Vault" />
      <QueryState query={vault} skeleton={<SettingGroupSkeleton rows={1} />}>
        {(info) => <WhereKept info={info} />}
      </QueryState>
      <QueryState query={accounts} skeleton={<SettingGroupSkeleton rows={3} />}>
        {(data) => <TokenList accounts={data.items} />}
      </QueryState>
    </>
  );
}

/** Where each kind of store and key source keeps things, in plain words. */
const STORES: Record<string, string> = {
  'built-in': 'In the vault itself',
  '1password': 'In 1Password, with a reference in the vault',
  vault: 'In HashiCorp Vault, with a reference in the vault',
};
const KEY_SOURCES: Record<string, string> = {
  'host-secret': 'A secret variable at your provider',
};

function place(info: VaultInfo): string {
  return info.kept_in === 'keychain' ? 'This computer’s keychain' : 'Your remote’s sealed file';
}

/** Where tokens are kept, from the vault itself. */
function WhereKept({ info }: { info: VaultInfo }) {
  return (
    <SettingGroup
      title="Where tokens are kept"
      description="Nobody at Hoista can see them, and agents never receive them. Project secrets go straight to your provider and aren’t kept here."
    >
      <SettingRow
        label="Vault"
        hint={
          info.kept_in === 'keychain'
            ? 'Your system’s own password store. Only Hoista on this computer reads it.'
            : 'Sealed with a key Hoista never stores, and opened only by the remote’s credential keeper.'
        }
      >
        <FixedValue>{place(info)}</FixedValue>
      </SettingRow>
      <SettingRow
        label="Each provider token"
        hint="Set with HOISTA_TOKEN_STORE where Hoista runs. Other places plug in here."
      >
        <FixedValue>{STORES[info.token_store] ?? info.token_store}</FixedValue>
      </SettingRow>
      {info.key_source && (
        <SettingRow label="Sealing key" hint="Set with HOISTA_KEY_SOURCE on the remote.">
          <FixedValue>{KEY_SOURCES[info.key_source] ?? info.key_source}</FixedValue>
        </SettingRow>
      )}
      {info.references && (
        <SettingRow
          label="1Password"
          hint="When you connect or reconnect an account, paste a 1Password secret reference (op://Vault/Item/field) instead of the token. The token stays in 1Password; Hoista reads it when it needs it."
        >
          <FixedValue>Supported</FixedValue>
        </SettingRow>
      )}
    </SettingGroup>
  );
}

function TokenList({ accounts }: { accounts: Account[] }) {
  const [removing, setRemoving] = useState<Account | null>(null);

  return (
    <SettingGroup
      title="Account tokens"
      description="One for each connected account. Replacing or removing one changes nothing at the provider."
    >
      {accounts.length === 0 && (
        <SettingRow label="No accounts connected yet" hint="Connect one from Accounts." />
      )}
      {accounts.map((account) => (
        <TokenRow
          key={account.id}
          account={account}
          onRemove={() => {
            setRemoving(account);
          }}
        />
      ))}
      {removing && (
        <RemoveTokenDialog
          account={removing}
          onClose={() => {
            setRemoving(null);
          }}
        />
      )}
    </SettingGroup>
  );
}

function TokenRow({ account, onRemove }: { account: Account; onRemove: () => void }) {
  const onTheirPage = Boolean(useConnectGuide(account.service).data?.sign_in);
  const [replacing, setReplacing] = useState(false);
  const name = `${serviceName(account.service)} · ${accountLabel(account)}`;

  return (
    <>
      <SettingRow
        icon={
          <span className={styles.logo}>
            <ServiceIcon id={account.service} size={16} />
          </span>
        }
        label={name}
        hint={tokenHint(account)}
      >
        <div className={styles.actions}>
          <Button
            aria-expanded={replacing}
            onClick={() => {
              setReplacing((open) => !open);
            }}
          >
            {onTheirPage ? 'Reconnect' : 'Replace'}
          </Button>
          <RowMenu
            label={`More for ${name}`}
            items={[{ label: 'Remove', danger: true, onSelect: onRemove }]}
          />
        </div>
      </SettingRow>
      {replacing && (
        <div className={styles.replace}>
          <ReplaceTokenPanel
            account={account}
            onReplaced={() => {
              setReplacing(false);
              showToast(`${name} reconnected`);
            }}
            onCancel={() => {
              setReplacing(false);
            }}
          />
        </div>
      )}
    </>
  );
}

/** The account's name; a bare ID (some tokens can't read the name) is shortened. */
function accountLabel(account: Account): string {
  return /^[0-9a-f]{8}-[0-9a-f-]{27}$/i.test(account.label)
    ? `workspace ${account.label.slice(0, 8)}`
    : account.label;
}

const STOPPED: Record<Exclude<Account['status'], 'ok'>, string> = {
  expired: 'Stopped working · expired or revoked at the provider',
  missing_scope: 'Stopped working · missing permissions Hoista needs',
  revoked: 'Disconnected · removed from this computer',
};

/** `Used by moodboard, waitlist-v2 · last worked 2h ago`, or what went wrong. */
function tokenHint(account: Account): string {
  if (account.status !== 'ok') return STOPPED[account.status];

  const usedBy = account.used_by.length ? `Used by ${listOf(account.used_by)}` : 'Not used yet';
  const where = account.kept_in === '1password' ? ' · in 1Password' : '';
  const lastWorked = account.last_worked_at
    ? ` · last worked ${timeAgo(account.last_worked_at)}`
    : '';
  return usedBy + where + lastWorked;
}
