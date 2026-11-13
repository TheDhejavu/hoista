import { useState } from 'react';
import type { Resource } from '@shared/api';
import { useProviderDetails } from '@ui/api/projects';
import { useProviderChange } from '@ui/api/deploys';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { cx } from '@ui/lib/cx';
import styles from './TcpEndpoints.module.css';
import shared from './domains.module.css';
import type { TcpProxy } from '@shared/api';
import { PortDialog } from './PortDialog';

/**
 * Public TCP endpoints per resource, for databases and anything that isn't
 * HTTP: how to reach it from outside, and opening or closing one.
 */
export function TcpEndpoints({
  resources,
  providerName,
}: {
  resources: Resource[];
  providerName: string;
}) {
  return (
    <section aria-labelledby="tcp-heading" className={styles.tcp}>
      <div className={shared.heading}>
        <h2 id="tcp-heading">TCP proxies</h2>
      </div>
      <p className={shared.dialogText}>
        Public addresses for databases and anything that doesn’t speak HTTP. Anyone with the address
        can try to connect, so keep passwords strong.
      </p>
      <div className={shared.frame}>
        {resources.map((resource) => (
          <TcpRows key={resource.id} resource={resource} providerName={providerName} />
        ))}
      </div>
    </section>
  );
}

function TcpRows({ resource, providerName }: { resource: Resource; providerName: string }) {
  const details = useProviderDetails(resource.id);
  const change = useProviderChange();
  const [opening, setOpening] = useState(false);
  const [closing, setClosing] = useState<TcpProxy | null>(null);
  const proxies = details.data?.tcp_proxies ?? [];
  return (
    <>
      {proxies.map((proxy) => (
        <div key={proxy.id} className={shared.row}>
          <span
            className={cx('mono', shared.name)}
            title={`${proxy.domain}:${String(proxy.proxy_port)}`}
          >
            {proxy.domain}:{proxy.proxy_port}
          </span>
          <span className={shared.role}>
            <span>{resource.name}</span>
            <span className={shared.note}>forwards to port {proxy.application_port}</span>
          </span>
          <Button
            variant="quietDanger"
            onClick={() => {
              setClosing(proxy);
            }}
          >
            Close
          </Button>
        </div>
      ))}
      {proxies.length === 0 && (
        <div className={shared.row}>
          <span className={shared.name}>{resource.name}</span>
          <span className={shared.role}>
            <span className={shared.note}>No public TCP endpoint</span>
          </span>
          <Button
            onClick={() => {
              setOpening(true);
            }}
          >
            Open an endpoint
          </Button>
        </div>
      )}
      {opening && (
        <PortDialog
          title={`Add a TCP proxy to ${resource.name}`}
          text={`${providerName} assigns a public host and port that forward TCP traffic to this port.`}
          onClose={() => {
            setOpening(false);
          }}
          onSave={(port) => {
            change.mutate(
              { resourceId: resource.id, body: { action: 'add_tcp_proxy', port } },
              {
                onSuccess: () => {
                  setOpening(false);
                },
              },
            );
          }}
          busy={change.isPending}
          error={change.error}
        />
      )}
      {closing && (
        <Dialog
          title={`Close ${closing.domain}:${String(closing.proxy_port)}?`}
          onClose={() => {
            setClosing(null);
          }}
          actions={
            <>
              <Button
                large
                onClick={() => {
                  setClosing(null);
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
                    {
                      resourceId: resource.id,
                      body: { action: 'remove_tcp_proxy', proxy_id: closing.id },
                    },
                    {
                      onSuccess: () => {
                        setClosing(null);
                      },
                    },
                  );
                }}
              >
                {change.isPending ? 'Closing' : 'Close it'}
              </Button>
            </>
          }
        >
          <p className={shared.dialogText}>
            Anything connecting from outside through this address stops working. Services inside{' '}
            {providerName} are not affected.
          </p>
          <ErrorMessage error={change.error} />
        </Dialog>
      )}
    </>
  );
}
