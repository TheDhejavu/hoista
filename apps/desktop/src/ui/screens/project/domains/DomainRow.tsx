import { MoreVertical } from 'lucide-react';
import { useState } from 'react';
import type { Domain } from '@shared/api';
import { useCheckDomain } from '@ui/api/projects';
import { useProviderChange } from '@ui/api/deploys';
import { openExternal } from '@ui/api/system';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { cx } from '@ui/lib/cx';
import { showToast } from '@ui/lib/toast';
import { useCopy } from '@ui/lib/useCopy';
import { usePopover } from '@ui/lib/usePopover';
import { tabStyles } from '../ProjectTab';
import styles from './DomainRow.module.css';
import shared from './domains.module.css';
import { PortDialog } from './PortDialog';

export function DomainRow({
  projectId,
  domain,
  providerName,
  watching,
  selected,
  onOpen,
}: {
  projectId: string;
  domain: Domain;
  providerName: string;
  watching: boolean;
  selected: boolean;
  onOpen: () => void;
}) {
  const { role, note } = describe(domain, providerName);
  const pill =
    // A provider's own address shows whether it works, like any other; that it's
    // built in is what the row says beside it.
    domain.status === 'active'
      ? { label: 'Active', dot: tabStyles.dotGood }
      : domain.status === 'pending_dns'
        ? { label: 'Pending', dot: tabStyles.dotWarn }
        : { label: 'Error', dot: tabStyles.dotBad };

  return (
    <div
      className={cx(shared.row, styles.rowButton, selected && styles.rowOpen)}
      role="button"
      tabIndex={0}
      aria-pressed={selected}
      onClick={onOpen}
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault();
          onOpen();
        }
      }}
    >
      <span className={cx('mono', shared.name)} title={domain.name}>
        {domain.name}
      </span>
      <span className={shared.role}>
        <span>{role}</span>
        <span className={shared.note}>{note}</span>
      </span>
      <span className={tabStyles.pill}>
        <span className={cx(tabStyles.dot, pill.dot)} />
        {pill.label}
      </span>
      <span
        onClick={(event) => {
          event.stopPropagation();
        }}
        onKeyDown={(event) => {
          event.stopPropagation();
        }}
        role="presentation"
      >
        <RowMenu projectId={projectId} domain={domain} watching={watching} />
      </span>
    </div>
  );
}

function describe(domain: Domain, providerName: string): { role: string; note: string } {
  if (domain.kind === 'default') {
    return {
      role: 'Default domain',
      note: `Assigned by ${providerName}`,
    };
  }
  if (domain.status === 'pending_dns') {
    const what = domain.setup?.records[0]
      ? `Add a ${domain.setup.records[0].type} record at your DNS provider`
      : 'Change the nameservers at your registrar';
    return { role: 'Pending DNS verification', note: what };
  }
  if (domain.status === 'error')
    return { role: 'DNS or SSL error', note: 'Rechecked every minute' };
  const ssl =
    domain.ssl_renews_in_days !== null
      ? `SSL certificate renews in ${String(domain.ssl_renews_in_days)} days`
      : domain.reaches
        ? `Routes to ${domain.reaches}${domain.port ? `, port ${String(domain.port)}` : ''}`
        : 'SSL certificate valid';
  if (domain.role === 'redirect' && domain.redirects_to) {
    return { role: `Redirects to ${domain.redirects_to}`, note: 'Redirect · SSL' };
  }
  return { role: domain.role === 'primary' ? 'Primary' : 'Custom domain', note: ssl };
}

function RowMenu({
  projectId,
  domain,
  watching,
}: {
  projectId: string;
  domain: Domain;
  watching: boolean;
}) {
  const { open, setOpen, ref } = usePopover<HTMLDivElement>();
  const check = useCheckDomain(projectId);
  const change = useProviderChange();
  const { copied, copy } = useCopy();
  const [dialog, setDialog] = useState<'port' | 'remove' | null>(null);
  const resourceId = watching ? domain.resource_id : undefined;

  return (
    <div ref={ref} className={styles.menuWrap}>
      <button
        type="button"
        className={styles.more}
        aria-label={`More for ${domain.name}`}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => {
          setOpen(!open);
        }}
      >
        <MoreVertical size={16} aria-hidden="true" />
      </button>
      {open && (
        <div className={styles.menu} role="menu">
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              void openExternal(`https://${domain.name}`);
              setOpen(false);
            }}
          >
            Open ↗
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              copy(`https://${domain.name}`);
            }}
          >
            {copied ? 'URL copied' : 'Copy URL'}
          </button>
          {domain.status !== 'active' && (
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                showToast(`Checking DNS for ${domain.name}`);
                check.mutate(domain.name, {
                  onSuccess: () => {
                    showToast(`DNS checked for ${domain.name}`);
                  },
                  onError: (error) => {
                    showToast(`Couldn’t check DNS: ${error.message}`, { tone: 'error' });
                  },
                });
                setOpen(false);
              }}
            >
              Check DNS now
            </button>
          )}
          {resourceId && (
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                setDialog('port');
                setOpen(false);
              }}
            >
              Change port
            </button>
          )}
          {resourceId && domain.kind === 'custom' && domain.status !== 'active' && (
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                showToast(`Asking for a new certificate for ${domain.name}`);
                change.mutate(
                  { resourceId, body: { action: 'retry_certificate', domain: domain.name } },
                  {
                    onSuccess: () => {
                      showToast(`Certificate requested for ${domain.name}`);
                    },
                    onError: (error) => {
                      showToast(`Couldn’t ask for a certificate: ${error.message}`, {
                        tone: 'error',
                      });
                    },
                  },
                );
                setOpen(false);
              }}
            >
              Issue the certificate again
            </button>
          )}
          {resourceId && (
            <button
              type="button"
              role="menuitem"
              className={styles.danger}
              onClick={() => {
                setDialog('remove');
                setOpen(false);
              }}
            >
              Remove
            </button>
          )}
        </div>
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
                disabled={change.isPending}
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
                {change.isPending ? 'Removing' : 'Remove'}
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
    </div>
  );
}
