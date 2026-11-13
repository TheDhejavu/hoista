/**
 * Adds a variable, or replaces or removes an existing one. The value is
 * typed once, sent to the provider, and never shown back. For a watched project the
 * variable goes to the chosen resource at its provider, which redeploys it.
 */
import { runsCode } from '../resourceKinds';
import { useId, useState } from 'react';
import type { Resource, Secret } from '@shared/api';
import { providerEntry, serviceName } from '@shared/providers';
import { useDeleteSecret, useSetSecret } from '@ui/api/projects';
import { SelectField } from '@ui/components/Select';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import styles from './VariableDialog.module.css';
import { useProjectNames } from '@ui/lib/useProjectNames';

export function VariableDialog({
  projectId,
  resources = [],
  provider = 'your provider',
  existing,
  onClose,
}: {
  projectId: string;
  /** For a watched project: the resources the variable can go to. Empty otherwise. */
  resources?: Resource[];
  provider?: string;
  /** The variable being replaced or filled in, or null to add a new one. */
  existing: Secret | null;
  onClose: () => void;
}) {
  const nameId = useId();
  const valueId = useId();
  const [name, setName] = useState(existing?.name ?? '');
  const [value, setValue] = useState('');
  const setSecret = useSetSecret(projectId);
  const projectName = useProjectNames();
  const remove = useDeleteSecret(projectId);
  const [confirming, setConfirming] = useState(false);
  const watching = resources.length > 0;
  const [resourceId, setResourceId] = useState(
    existing?.resource_id ??
      resources.find((resource) => runsCode(resource.kind))?.id ??
      resources[0]?.id ??
      '',
  );
  // Watched: a variable can go on one service or be shared by the whole project.
  const [shared, setShared] = useState(existing?.shared === true);
  const target = resources.find((resource) => resource.id === resourceId);
  // The chosen resource's provider: its name, and how its variables work.
  const providerName = target ? serviceName(target.provider) : provider;
  const how = target ? providerEntry(target.provider)?.variables : undefined;
  const sharing = how?.shared === true;
  const redeploys = !watching || how?.redeploys === true;

  const isPublic = name.startsWith('VITE_') || name.startsWith('NEXT_PUBLIC_');

  function save(): void {
    setSecret.mutate(
      { name, value, ...(watching && { resourceId, shared: shared && sharing }) },
      { onSuccess: onClose },
    );
  }

  return (
    <Dialog
      eyebrow={`${projectName(projectId)} · Environment`}
      title={
        existing ? `${existing.missing ? 'Add' : 'Replace'} ${existing.name}` : 'Add a variable'
      }
      onClose={onClose}
      actions={
        <>
          {watching && existing && !existing.missing && (
            <Button
              large
              variant="ghost"
              disabled={remove.isPending}
              onClick={() => {
                // Removing redeploys the service without it: ask once more.
                if (!confirming) {
                  setConfirming(true);
                  return;
                }
                remove.mutate(
                  { name: existing.name, resourceId, shared: existing.shared === true },
                  { onSuccess: onClose },
                );
              }}
            >
              {remove.isPending
                ? 'Removing'
                : confirming
                  ? `Remove ${existing.name} and redeploy`
                  : 'Remove'}
            </Button>
          )}
          <Button large onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            large
            disabled={!name || !value || setSecret.isPending}
            onClick={save}
          >
            {setSecret.isPending ? 'Saving' : redeploys ? 'Save and redeploy' : 'Save'}
          </Button>
        </>
      }
    >
      <div className={styles.form}>
        {watching && !existing && (
          <SelectField
            label="Set it on"
            value={shared && sharing ? '' : resourceId}
            options={[
              ...resources.map((resource) => ({
                value: resource.id,
                label: resource.name,
                group: serviceName(resource.provider),
              })),
              ...(sharing ? [{ value: '', label: 'Shared with every service' }] : []),
            ]}
            onChange={(next) => {
              setShared(next === '');
              if (next) setResourceId(next);
            }}
          />
        )}
        <div className={styles.field}>
          <label htmlFor={nameId} className={styles.label}>
            Name
          </label>
          <input
            id={nameId}
            className={styles.input}
            value={name}
            readOnly={existing !== null}
            placeholder="RESEND_API_KEY"
            spellCheck={false}
            onChange={(event) => {
              setName(event.target.value.toUpperCase());
            }}
          />
        </div>
        <div className={styles.field}>
          <label htmlFor={valueId} className={styles.label}>
            Value
          </label>
          <input
            id={valueId}
            className={styles.input}
            type={isPublic ? 'text' : 'password'}
            value={value}
            autoComplete="off"
            spellCheck={false}
            onChange={(event) => {
              setValue(event.target.value);
            }}
          />
          <p className={styles.hint}>
            {isPublic ? (
              <span className={styles.warning}>
                This name is bundled into the site, so anyone can read the value. Only use it for
                public keys.
              </span>
            ) : watching && shared && sharing ? (
              `Sent straight to ${providerName} as a shared variable: every service in the project can use it. Hoista doesn't keep the value.`
            ) : watching ? (
              `Sent straight to ${providerName}${target ? ` for ${target.name}` : ''}${redeploys ? ', which redeploys it to apply the change' : ''}. Hoista doesn't keep the value.`
            ) : (
              'Sent to your provider as a secret. Hoista keeps only a fingerprint, so you can see when it changes.'
            )}
          </p>
        </div>
        <ErrorMessage error={setSecret.error ?? remove.error} />
      </div>
    </Dialog>
  );
}
