import { useId, useState } from 'react';
import type { Resource } from '@shared/api';
import { useProviderChange } from '@ui/api/deploys';
import { SelectField } from '@ui/components/Select';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import styles from './WatchedAddDialog.module.css';
import shared from './domains.module.css';

/** Add an address at the provider: your own domain, or a free one from the provider. */
export function WatchedAddDialog({
  resources,
  providerName,
  onClose,
}: {
  resources: Resource[];
  providerName: string;
  onClose: () => void;
}) {
  const change = useProviderChange();
  const [resourceId, setResourceId] = useState(resources[0]?.id ?? '');
  const [domain, setDomain] = useState('');
  const [port, setPort] = useState('');
  const domainId = useId();
  const portId = useId();
  const portOk = port === '' || (/^\d+$/.test(port) && Number(port) > 0 && Number(port) < 65536);
  return (
    <Dialog
      title="Add a domain"
      eyebrow={`At ${providerName}`}
      onClose={onClose}
      actions={
        <>
          <Button large onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            large
            disabled={!resourceId || !portOk || change.isPending}
            onClick={() => {
              change.mutate(
                {
                  resourceId,
                  body: {
                    action: 'add_domain',
                    ...(domain.trim() && { domain: domain.trim().toLowerCase() }),
                    ...(port && { port: Number(port) }),
                  },
                },
                { onSuccess: onClose },
              );
            }}
          >
            {change.isPending
              ? 'Adding'
              : domain.trim()
                ? 'Add domain'
                : `Generate ${providerName} domain`}
          </Button>
        </>
      }
    >
      <div className={styles.form}>
        {resources.length > 1 && (
          <SelectField
            label="Resource"
            value={resourceId}
            options={resources.map((resource) => ({ value: resource.id, label: resource.name }))}
            onChange={setResourceId}
          />
        )}
        <label htmlFor={domainId} className={shared.fieldLabel}>
          Your domain
        </label>
        <input
          id={domainId}
          className={shared.input}
          placeholder="api.example.com"
          value={domain}
          onChange={(event) => {
            setDomain(event.target.value);
          }}
        />
        <p className={shared.dialogText}>
          Leave it empty to generate a {providerName} domain. For a custom domain, Hoista then shows
          the DNS record to add.
        </p>
        <label htmlFor={portId} className={shared.fieldLabel}>
          Port (optional)
        </label>
        <input
          id={portId}
          className={shared.input}
          inputMode="numeric"
          placeholder="The port your service listens on"
          value={port}
          onChange={(event) => {
            setPort(event.target.value.trim());
          }}
        />
        <ErrorMessage error={change.error} />
      </div>
    </Dialog>
  );
}
