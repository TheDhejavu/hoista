/**
 * "Add server" on a project's overview: a new Linux server at a provider that
 * makes them (Hetzner), set up so agents reach it through Hoista. The account,
 * location and server type are dropdowns that show None until picked; the
 * choices come from the provider's own lists. The dialog stays open while the
 * server is made and then says what happened.
 */
import { useState } from 'react';
import type { Account } from '@shared/api';
import { providerEntry } from '@shared/providers';
import type { ServerChoice, ServerChoices } from '@shared/providers/types';
import { useAccounts } from '@ui/api/accounts';
import { useProviderRead } from '@ui/api/providerOperations';
import { useCan } from '@ui/api/providers';
import { useAddServer, useNewServer } from '@ui/api/servers';
import { Button } from '@ui/components/Button';
import { Dialog, DialogText } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { TextField } from '@ui/components/Field';
import { Select, type Option } from '@ui/components/Select';
import { Skeleton } from '@ui/components/Skeleton';
import styles from './AddServer.module.css';

/** Accounts whose provider makes servers. */
function serverAccounts(
  accounts: Account[],
  can: ReturnType<typeof useCan>,
): { account: Account; choices: ServerChoices }[] {
  return accounts.flatMap((account) => {
    const choices = providerEntry(account.service)?.servers;
    return choices && account.status === 'ok' && can(account.service, 'new_server')
      ? [{ account, choices }]
      : [];
  });
}

export function AddServer({ projectId }: { projectId: string }) {
  const [open, setOpen] = useState(false);
  const can = useCan();
  const accounts = serverAccounts(useAccounts().data?.items ?? [], can);
  if (accounts.length === 0) return null;
  return (
    <>
      <Button
        onClick={() => {
          setOpen(true);
        }}
      >
        Add server
      </Button>
      {open && (
        <AddServerDialog
          projectId={projectId}
          accounts={accounts}
          onClose={() => {
            setOpen(false);
          }}
        />
      )}
    </>
  );
}

const NONE = '';

function listOf(answer: unknown, list: string): ServerChoice[] {
  if (typeof answer !== 'object' || answer === null) return [];
  const items = (answer as Record<string, unknown>)[list];
  return Array.isArray(items) ? (items as ServerChoice[]) : [];
}

function AddServerDialog({
  projectId,
  accounts,
  onClose,
}: {
  projectId: string;
  accounts: { account: Account; choices: ServerChoices }[];
  onClose: () => void;
}) {
  const [accountId, setAccountId] = useState(NONE);
  const [location, setLocation] = useState(NONE);
  const [serverType, setServerType] = useState(NONE);
  const [name, setName] = useState('');
  const [volume, setVolume] = useState<number | null>(null);
  const [backups, setBackups] = useState(false);
  const add = useAddServer();
  const madeId = add.data?.resources.find((resource) => resource.kind === 'server')?.id ?? null;
  const made = useNewServer(projectId, madeId);
  const chosen = accounts.find((item) => item.account.id === accountId);
  const target = chosen ? { kind: 'account' as const, id: chosen.account.id } : null;
  const locations = useProviderRead(
    target,
    chosen?.choices.locations.op ?? '',
    chosen?.choices.locations.args ?? {},
  );
  const types = useProviderRead(
    target,
    chosen?.choices.types.op ?? '',
    chosen?.choices.types.args ?? {},
  );
  const noun = chosen?.choices.noun ?? 'Server';
  const nameOk = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/.test(name);
  const ready = chosen !== undefined && location !== NONE && serverType !== NONE && nameOk;

  if (add.isSuccess) {
    return (
      <Dialog
        title={`${noun} on its way`}
        onClose={onClose}
        actions={
          <Button variant="accent" onClick={onClose}>
            Done
          </Button>
        }
      >
        <DialogText>
          <p>{add.data.summary}</p>
        </DialogText>
        <Progress status={made.data?.status} />
      </Dialog>
    );
  }

  const accountOptions: Option<string>[] = [
    { value: NONE, label: 'None' },
    ...accounts.map(({ account }) => ({
      value: account.id,
      label: account.label,
      note: providerEntry(account.service)?.name,
    })),
  ];
  const locationOptions: Option<string>[] = [
    { value: NONE, label: 'None' },
    ...(chosen
      ? listOf(locations.data, chosen.choices.locations.list).map((item) => ({
          value: chosen.choices.locations.value(item),
          label: chosen.choices.locations.label(item),
        }))
      : []),
  ];
  const typeOptions: Option<string>[] = [
    { value: NONE, label: 'None' },
    ...(chosen && location !== NONE
      ? listOf(types.data, chosen.choices.types.list)
          .filter((item) => chosen.choices.types.offered(item, location))
          .map((item) => ({
            value: chosen.choices.types.value(item),
            label: chosen.choices.types.label(item),
          }))
      : []),
  ];

  return (
    <Dialog
      title="Add a server"
      eyebrow="Ubuntu 24.04 with Docker, a firewall that lets in only SSH, HTTP and HTTPS, and security updates on. Only Hoista can sign in."
      onClose={onClose}
      actions={
        <>
          <Button onClick={onClose} disabled={add.isPending}>
            Cancel
          </Button>
          <Button
            variant="accent"
            disabled={!ready}
            busy={add.isPending}
            onClick={() => {
              add.mutate({
                projectId,
                accountId,
                name,
                serverType,
                location,
                volumeGb: volume,
                backups,
              });
            }}
          >
            {add.isPending ? `Making ${noun.toLowerCase()}` : `Add ${noun.toLowerCase()}`}
          </Button>
        </>
      }
    >
      <div className={styles.form}>
        <div className={styles.field}>
          <span className={styles.label}>Account</span>
          <Select
            label="Account"
            value={accountId}
            options={accountOptions}
            disabled={add.isPending}
            onChange={(id) => {
              setAccountId(id);
              setLocation(NONE);
              setServerType(NONE);
            }}
          />
        </div>
        {chosen && (
          <>
            <div className={styles.field}>
              <span className={styles.label}>Location</span>
              {locations.isPending ? (
                <Skeleton height={32} />
              ) : (
                <Select
                  label="Location"
                  value={location}
                  options={locationOptions}
                  disabled={add.isPending}
                  onChange={(next) => {
                    setLocation(next);
                    setServerType(NONE);
                  }}
                />
              )}
            </div>
            <div className={styles.field}>
              <span className={styles.label}>{noun} type</span>
              {types.isPending ? (
                <Skeleton height={32} />
              ) : (
                <Select
                  label={`${noun} type`}
                  value={serverType}
                  options={typeOptions}
                  disabled={add.isPending || location === NONE}
                  onChange={setServerType}
                />
              )}
            </div>
            {chosen.choices.volumeSizes && (
              <div className={styles.field}>
                <span className={styles.label}>Volume</span>
                <Select
                  label="Volume"
                  value={volume}
                  options={[
                    { value: null, label: 'None' },
                    ...chosen.choices.volumeSizes.map((gb) => ({
                      value: gb,
                      label: `${String(gb)} GB, mounted on the server`,
                    })),
                  ]}
                  disabled={add.isPending}
                  onChange={setVolume}
                />
              </div>
            )}
            {chosen.choices.backups && (
              <div className={styles.field}>
                <span className={styles.label}>Backups</span>
                <Select
                  label="Backups"
                  value={backups}
                  options={[
                    { value: false, label: 'Off' },
                    { value: true, label: 'On' },
                  ]}
                  disabled={add.isPending}
                  onChange={setBackups}
                />
                <span className={styles.hint}>{chosen.choices.backups.note}</span>
              </div>
            )}
            <TextField
              label="Name"
              value={name}
              placeholder="web-1"
              hint="Lowercase letters, digits and hyphens. It becomes the server's hostname."
              onChange={setName}
            />
          </>
        )}
        <ErrorMessage error={locations.error ?? types.error ?? add.error} />
      </div>
    </Dialog>
  );
}

/**
 * Where a new server is, from what the project shows: made, then starting
 * and setting itself up, then ready (or its setup failed). Closing the
 * dialog doesn't stop it.
 */
function Progress({ status }: { status: string | undefined }) {
  const ready = status === 'live';
  const failed = status === 'failing';
  const steps: { label: string; state: 'done' | 'now' | 'later' | 'failed' }[] = [
    { label: 'Made at the provider', state: 'done' },
    {
      label: 'Starting and setting itself up, a few minutes',
      state: ready ? 'done' : failed ? 'failed' : 'now',
    },
    { label: 'Ready for agents', state: ready ? 'done' : 'later' },
  ];
  return (
    <ol className={styles.steps} aria-live="polite">
      {steps.map((step) => (
        <li key={step.label} className={styles[step.state]}>
          <span className={styles.mark} aria-hidden="true" />
          {step.state === 'failed'
            ? 'Its setup failed. The server’s Activity on its Settings tab says why.'
            : step.label}
        </li>
      ))}
    </ol>
  );
}
