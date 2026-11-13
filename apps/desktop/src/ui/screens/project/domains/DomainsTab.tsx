/**
 * Domains: every address the app answers on, what each one does and whether
 * it works. A domain still waiting for DNS gets a card with exactly what to
 * add and where; Hoista keeps checking until it works.
 */
import { runsCode } from '../resourceKinds';
import { useState } from 'react';
import { providerEntry, serviceName } from '@shared/providers';
import { useProject, useDomains } from '@ui/api/projects';
import { Button } from '@ui/components/Button';
import { QueryState } from '@ui/components/QueryState';
import { SkeletonRows } from '@ui/components/Skeleton';
import { cx } from '@ui/lib/cx';
import { useRequiredParam } from '@ui/lib/useRequiredParam';
import { ProjectTab } from '../ProjectTab';
import styles from './DomainsTab.module.css';
import shared from './domains.module.css';
import { DomainRow } from './DomainRow';
import { TcpEndpoints } from './TcpEndpoints';
import { FinishCard } from './FinishCard';
import { DomainPanel } from './DomainPanel';
import { WatchedAddDialog } from './WatchedAddDialog';
import { AddDomainDialog } from './AddDomainDialog';

export function DomainsTab() {
  const projectId = useRequiredParam('projectId');
  const domains = useDomains(projectId);
  const detail = useProject(projectId).data;
  const project = detail?.project;
  const provider = project?.providers[0];
  // Each domain's own provider: in a project across providers, the Vercel address is Vercel's.
  const providerOf = (resourceId: string | null | undefined, fallback: string): string => {
    const owner =
      detail?.resources.find((resource) => resource.id === resourceId)?.provider ?? provider;
    return owner ? serviceName(owner) : fallback;
  };
  // Watched projects: domains, ports, certificates and TCP endpoints are changed at the provider.
  const watching = project?.mode === 'watching';
  // Only resources whose provider can open a TCP endpoint in front of them.
  const tcpResources = (detail?.resources ?? []).filter(
    (resource) => providerEntry(resource.provider)?.tcpEndpoints,
  );
  // A watched project's domains are added only where its provider takes that through Hoista.
  const codeResources = (detail?.resources ?? []).filter(
    (resource) =>
      runsCode(resource.kind) &&
      (!watching || providerEntry(resource.provider)?.domainActions?.add === true),
  );
  const [adding, setAdding] = useState(false);
  const [openName, setOpenName] = useState<string | null>(null);
  const items = domains.data?.items ?? [];
  const pending = items.find((domain) => domain.status === 'pending_dns') ?? null;
  // The domain shown beside the list: the one clicked, else one waiting for
  // DNS, else the first. Always one, so opening another never moves the page.
  const opened = items.find((domain) => domain.name === openName) ?? pending ?? items[0] ?? null;

  return (
    <ProjectTab>
      <div className={cx(styles.layout, opened && styles.withCard)}>
        <section aria-labelledby="domains-heading" className={styles.main}>
          <div className={shared.heading}>
            <h2 id="domains-heading">Domains</h2>
            {(!watching || codeResources.length > 0) && (
              <Button
                variant="accent"
                large
                onClick={() => {
                  setAdding(true);
                }}
              >
                Add a domain
              </Button>
            )}
          </div>
          <div className={shared.frame}>
            <QueryState query={domains} skeleton={<SkeletonRows count={2} columns={3} />}>
              {(page) =>
                page.items.length === 0 ? (
                  <p className={styles.empty}>
                    {watching
                      ? `${provider ? serviceName(provider) : 'The provider'} reports no domains for this project.`
                      : 'No domains yet.'}
                  </p>
                ) : (
                  page.items.map((domain) => (
                    <DomainRow
                      key={domain.name}
                      projectId={projectId}
                      domain={domain}
                      providerName={providerOf(domain.resource_id, 'Provider')}
                      watching={watching}
                      selected={domain.name === opened?.name}
                      onOpen={() => {
                        setOpenName(domain.name);
                      }}
                    />
                  ))
                )
              }
            </QueryState>
          </div>
          {watching && tcpResources.length > 0 && (
            <TcpEndpoints
              resources={tcpResources}
              providerName={provider ? serviceName(provider) : 'the provider'}
            />
          )}
        </section>
        {opened &&
          (opened.status === 'pending_dns' && opened.setup ? (
            <FinishCard projectId={projectId} domain={opened} />
          ) : (
            <DomainPanel
              domain={opened}
              watching={watching}
              providerName={providerOf(opened.resource_id, 'the provider')}
              actions={
                providerEntry(
                  detail?.resources.find((resource) => resource.id === opened.resource_id)
                    ?.provider ?? '',
                )?.domainActions ?? {}
              }
            />
          ))}
      </div>

      {adding && watching && (
        <WatchedAddDialog
          resources={codeResources}
          providerName={provider ? serviceName(provider) : 'the provider'}
          onClose={() => {
            setAdding(false);
          }}
        />
      )}
      {adding && !watching && (
        <AddDomainDialog
          projectId={projectId}
          onClose={() => {
            setAdding(false);
          }}
        />
      )}
    </ProjectTab>
  );
}
