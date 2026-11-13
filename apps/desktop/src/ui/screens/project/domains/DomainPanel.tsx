import { useId, useState } from 'react';
import type { Domain } from '@shared/api';
import type { DomainActions } from '@shared/providers/types';
import { useProviderChange } from '@ui/api/deploys';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { cx } from '@ui/lib/cx';
import { tabStyles } from '../ProjectTab';
import styles from './DomainPanel.module.css';
import shared from './domains.module.css';
import { PortDialog } from './PortDialog';

/**
 * One domain, opened: where it goes, its certificate, every DNS record it
 * uses with what the DNS host has now, and what can be changed.
 */
export function DomainPanel({
  domain,
  watching,
  providerName,
  actions,
}: {
  domain: Domain;
  watching: boolean;
  providerName: string;
  actions: DomainActions;
}) {
  const change = useProviderChange();
  const [dialog, setDialog] = useState<'port' | 'remove' | 'rename' | null>(null);
  const builtIn = domain.kind === 'default';
  const canPort = actions.port === true;
  const canRename = builtIn && actions.rename === true;
  const canRetry = !builtIn && domain.certificate !== 'Valid' && actions.certificate === true;
  // A provider's own address stays with the resource unless the provider can rename it.
  const canRemove = actions.remove === true && (!builtIn || actions.rename === true);
  const resourceId =
    watching && (canPort || canRename || canRetry || canRemove) ? domain.resource_id : undefined;
  const dns = domain.dns ?? [];
  const pill =
    domain.status === 'active'
      ? { label: 'Active', dot: tabStyles.dotGood }
      : domain.status === 'pending_dns'
        ? { label: 'Pending', dot: tabStyles.dotWarn }
        : { label: 'Error', dot: tabStyles.dotBad };
  const facts: [string, string][] = [
    ...(domain.reaches ? [['Service', domain.reaches] as [string, string]] : []),
    ...(domain.port ? [['Port', String(domain.port)] as [string, string]] : []),
    ...(builtIn
      ? []
      : [
          [
            'Certificate',
            domain.certificate === 'Valid'
              ? `Valid · ${providerName} renews it`
              : (domain.certificate ?? 'Valid'),
          ] as [string, string],
        ]),
    [
      'Role',
      builtIn ? 'Default domain' : domain.role === 'primary' ? 'Primary domain' : 'Custom domain',
    ],
  ];

  return (
    <aside className={shared.card} aria-labelledby="domain-heading">
      <div className={styles.panelHead}>
        <h2 id="domain-heading" className="mono">
          {domain.name}
        </h2>
        <span className={tabStyles.pill}>
          <span className={cx(tabStyles.dot, pill.dot)} />
          {pill.label}
        </span>
      </div>
      <p className={shared.cardText}>
        {builtIn
          ? `The default domain ${providerName} assigns to this service. It stays active while the service runs.`
          : `${providerName}’s edge network routes this domain to ${domain.reaches ?? 'the project'}. Point it at ${providerName} with the DNS record shown; there’s no fixed IP address.`}
      </p>

      <h3 className={styles.panelLabel}>Where it goes</h3>
      <dl className={styles.facts}>
        {facts.map(([term, value]) => (
          <div key={term}>
            <dt>{term}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>

      {dns.length > 0 && (
        <>
          <h3 className={styles.panelLabel}>DNS records at your DNS provider</h3>
          <div className={shared.recordFrame}>
            <table className={cx(shared.records, shared.checks)}>
              <thead>
                <tr>
                  <th scope="col">Type</th>
                  <th scope="col">Name</th>
                  <th scope="col">Value</th>
                  <th scope="col">
                    <span className={tabStyles.visuallyHidden}>Matches</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {dns.map((record) => (
                  <tr key={`${record.type}-${record.name}`}>
                    <td>{record.type}</td>
                    <td title={record.name}>{record.name.split('.')[0]}</td>
                    <td className={shared.value} title={record.value}>
                      {record.value}
                      {!record.ok && (
                        <span className={styles.isNow}>
                          now {record.current ? record.current : 'missing'}
                        </span>
                      )}
                    </td>
                    <td className={record.ok ? styles.tick : styles.cross}>
                      {record.ok ? '✓' : '✗'}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <p className={styles.small}>
            {dns.every((record) => record.ok)
              ? 'Every record matches what your DNS provider has now.'
              : 'Change the records marked ✗ at your DNS provider to the value shown. Hoista rechecks every minute.'}
          </p>
        </>
      )}

      {resourceId && (
        <>
          <h3 className={styles.panelLabel}>Change it</h3>
          <div className={styles.panelActions}>
            {canPort && (
              <Button
                onClick={() => {
                  setDialog('port');
                }}
              >
                Change port
              </Button>
            )}
            {canRename && (
              <Button
                onClick={() => {
                  setDialog('rename');
                }}
              >
                Rename
              </Button>
            )}
            {canRetry && (
              <Button
                busy={change.isPending}
                onClick={() => {
                  change.mutate({
                    resourceId,
                    body: { action: 'retry_certificate', domain: domain.name },
                  });
                }}
              >
                Retry certificate
              </Button>
            )}
            {canRemove && (
              <Button
                variant="quietDanger"
                onClick={() => {
                  setDialog('remove');
                }}
              >
                Remove
              </Button>
            )}
          </div>
          {!builtIn && (
            <p className={styles.guide}>
              <b>Moving to a new domain?</b> A custom domain can’t be renamed. Add the new one, set
              its DNS record, wait until it’s Active, then remove this one.
            </p>
          )}
          <ErrorMessage error={dialog === null ? change.error : null} />
        </>
      )}

      {resourceId && dialog === 'port' && (
        <PortDialog
          title={`Change the port for ${domain.name}`}
          text="The port your service listens on. Visitors to this address reach that port."
          onClose={() => {
            setDialog(null);
          }}
          onSave={(port) => {
            change.mutate(
              { resourceId, body: { action: 'update_domain', domain: domain.name, port } },
              {
                onSuccess: () => {
                  setDialog(null);
                },
              },
            );
          }}
          busy={change.isPending}
          error={change.error}
        />
      )}
      {resourceId && dialog === 'rename' && (
        <RenameDialog
          domain={domain.name}
          busy={change.isPending}
          error={change.error}
          onClose={() => {
            setDialog(null);
          }}
          onSave={(to) => {
            change.mutate(
              { resourceId, body: { action: 'rename_domain', domain: domain.name, to } },
              {
                onSuccess: () => {
                  setDialog(null);
                },
              },
            );
          }}
        />
      )}
      {resourceId && dialog === 'remove' && (
        <Dialog
          title={`Remove ${domain.name}?`}
          onClose={() => {
            setDialog(null);
          }}
          actions={
            <>
              <Button
                large
                onClick={() => {
                  setDialog(null);
                }}
              >
                Cancel
              </Button>
              <Button
                variant="danger"
                large
                busy={change.isPending}
                busyLabel="Removing"
                onClick={() => {
                  change.mutate(
                    { resourceId, body: { action: 'remove_domain', domain: domain.name } },
                    {
                      onSuccess: () => {
                        setDialog(null);
                      },
                    },
                  );
                }}
              >
                Remove
              </Button>
            </>
          }
        >
          <p className={shared.dialogText}>
            Visitors to {domain.name} stop reaching it. Your DNS records stay as they are.
          </p>
          <ErrorMessage error={change.error} />
        </Dialog>
      )}
    </aside>
  );
}

/** A new first label for the provider's own address: `api-prod` in `api-prod.up.railway.app`. */
function RenameDialog({
  domain,
  busy,
  error,
  onClose,
  onSave,
}: {
  domain: string;
  busy: boolean;
  error: Error | null;
  onClose: () => void;
  onSave: (to: string) => void;
}) {
  const inputId = useId();
  const [label, rest = ''] = domain.split(/\.(.*)/s);
  const [name, setName] = useState(label ?? '');
  const valid = /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(name);
  return (
    <Dialog
      title={`Rename ${domain}`}
      onClose={onClose}
      actions={
        <>
          <Button large onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            large
            disabled={!valid || name === label}
            busy={busy}
            busyLabel="Renaming"
            onClick={() => {
              onSave(`${name}.${rest}`);
            }}
          >
            Rename
          </Button>
        </>
      }
    >
      <label htmlFor={inputId} className={shared.dialogText}>
        New name
      </label>
      <div className={styles.renameField}>
        <input
          id={inputId}
          className={styles.renameInput}
          value={name}
          spellCheck={false}
          onChange={(event) => {
            setName(event.target.value.toLowerCase());
          }}
        />
        <span className="mono">.{rest}</span>
      </div>
      <p className={shared.dialogText}>
        The old address stops working straight away. Lowercase letters, digits and dashes.
      </p>
      <ErrorMessage error={error} />
    </Dialog>
  );
}
