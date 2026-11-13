/**
 * One watched resource at its provider (a Railway service or database), with
 * everything the provider reports and the common things to do to it: redeploy,
 * restart or stop it, change how it builds and runs, and see what it
 * connects to, its addresses, disks and recent deploys. Anything else the
 * provider can do is one click away on the provider's tab.
 */
import { commitUrl, splitSource } from '@ui/lib/links';
import { RowMenu } from '@ui/components/RowMenu';
import { ResourceTabBody, ResourceTabs, type ResourceTab } from './ResourceTabs';
import { useCan } from '@ui/api/providers';
import { DeleteAtProvider } from '../DeleteAtProvider';
import { RemoveFromProject } from '../RemoveFromProject';
import { AddressLine, SourceLine } from './SourceLine';
import { ExternalLink } from '@ui/components/ExternalLink';
import { useQueries } from '@tanstack/react-query';
import { useState, type ReactNode } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import type { ResourceDetail } from '@shared/api';
import { serviceName } from '@shared/providers';
import { useProject, providerDetailsQuery, useProviderDetails } from '@ui/api/projects';
import { useProviderChange } from '@ui/api/deploys';
import { openExternal } from '@ui/api/system';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { cx } from '@ui/lib/cx';
import { regionName } from '@ui/lib/regions';
import { statusLook } from '@ui/lib/status';
import { formatMegabytes, timeAgo } from '@ui/lib/format';
import { ProviderStats } from '../overview/ProviderStats';
import { resourceTerm } from '../resourceKinds';
import styles from './ResourceScreen.module.css';
import type { ProviderDetails } from '@shared/api';
import { CopyText, Rows, Section } from './ResourceSection';
import { ConfigureDialog } from './ConfigureDialog';

export function ProviderResourceView({
  projectId,
  projectName,
  detail,
  tab = 'overview',
}: {
  projectId: string;
  projectName: string;
  detail: ResourceDetail;
  tab?: ResourceTab;
}) {
  const { resource } = detail;
  const provider = serviceName(resource.provider);
  const details = useProviderDetails(resource.id);
  const projectResources = useProject(projectId).data?.resources ?? [];
  const change = useProviderChange();
  const [editing, setEditing] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [dialog, setDialog] = useState<'remove' | 'delete' | null>(null);
  const navigate = useNavigate();
  const can = useCan();
  const projectSource = useProject(projectId).data?.project.source;
  const linked = projectSource?.kind === 'github' ? splitSource(projectSource.ref) : null;
  // The deployment serving now: what the source line under the name points at.
  const newest = details.data?.deployments.find((item) => item.state === 'succeeded');
  const { label, tone } = statusLook(resource.status);
  const isDatabase = resource.kind === 'database';
  const running = change.isPending ? (change.variables.body.action as string) : null;

  // What uses this one: other resources whose variables point at it by name.
  const others = projectResources.filter(
    (other) => other.id !== resource.id && other.provider === resource.provider,
  );
  const otherProviders = useQueries({
    queries: others.map((other) => providerDetailsQuery(other.id)),
  });
  const usedBy = others.flatMap((other, index) =>
    (otherProviders[index]?.data?.references ?? [])
      .filter((reference) => reference.service === resource.name)
      .map((reference) => ({ resource: other, via: reference.variable })),
  );
  const uses = details.data?.references ?? [];
  const config = details.data?.config ?? null;

  const publicCount = (details.data?.domains.length ?? 0) + (details.data?.tcp_proxies.length ?? 0);
  // Railway's connection variables on a database, for example DATABASE_URL
  // and DATABASE_PUBLIC_URL. Names only: the values hold the password.
  const connectVariables = (details.data?.variable_names ?? []).filter((name) =>
    /^(DATABASE|REDIS|MONGO|MYSQL)(_PUBLIC)?_URL$/.test(name),
  );

  const act = (action: string) => {
    change.mutate({ resourceId: resource.id, body: { action } });
  };

  return (
    <>
      <div className={styles.title}>
        <span className={styles.logo}>
          <ServiceIcon id={resource.provider} size={20} />
        </span>
        <div className={styles.names}>
          <div className={styles.nameRow}>
            <h1>{resource.name}</h1>
            <span className={cx(styles.pill, resource.status === 'failing' && styles.pillBad)}>
              <span className={cx(styles.dot, styles[tone])} />
              {label}
            </span>
          </div>
          <div className={cx('mono', styles.summary)}>
            {`${provider} · ${resourceTerm(resource)}`}
          </div>
          <AddressLine url={resource.url} />
          <SourceLine
            repo={newest?.repo ?? config?.repo ?? linked?.repo ?? projectSource?.git?.repository}
            branch={newest?.branch ?? projectSource?.git?.branch}
            commit={newest?.commit ?? linked?.commit ?? projectSource?.git?.commit}
            folder={projectSource?.kind === 'folder' ? projectSource.ref : null}
          />
        </div>
        <div className={styles.actions}>
          {!isDatabase && (
            <Button
              busy={running === 'redeploy'}
              busyLabel="Redeploying"
              disabled={change.isPending}
              onClick={() => {
                act('redeploy');
              }}
            >
              Redeploy
            </Button>
          )}
          <Button
            onClick={() => {
              void openExternal(detail.console_url);
            }}
          >
            Open in {provider} ↗
          </Button>
          <RowMenu
            label={`More for ${resource.name}`}
            items={[
              {
                label: running === 'restart' ? 'Restarting' : 'Restart',
                disabled: change.isPending,
                onSelect: () => {
                  act('restart');
                },
              },
              {
                label: `Everything on ${provider}`,
                onSelect: () => {
                  void navigate(
                    `/projects/${projectId}/provider?resource=${encodeURIComponent(resource.id)}`,
                  );
                },
              },
              {
                label: 'Move to another project',
                onSelect: () => {
                  void navigate(`/projects/${projectId}/resources/${resource.id}/settings`);
                },
              },
              {
                label: 'Remove from project',
                onSelect: () => {
                  setDialog('remove');
                },
              },
              ...(isDatabase
                ? []
                : [
                    {
                      label: 'Stop',
                      danger: true,
                      disabled: change.isPending,
                      onSelect: () => {
                        setStopping(true);
                      },
                    },
                  ]),
              ...(can(resource.provider, 'delete')
                ? [
                    {
                      label: `Delete on ${provider}`,
                      danger: true,
                      onSelect: () => {
                        setDialog('delete');
                      },
                    },
                  ]
                : []),
            ]}
          />
        </div>
        <RemoveFromProject
          resource={resource}
          projectName={projectName}
          last={projectResources.length <= 1}
          opened={dialog === 'remove'}
          onClosed={() => {
            setDialog(null);
          }}
          onRemoved={() => {
            void navigate(`/projects/${projectId}/map`);
          }}
        />
        <DeleteAtProvider
          resource={resource}
          last={projectResources.length <= 1}
          opened={dialog === 'delete'}
          onClosed={() => {
            setDialog(null);
          }}
        />
      </div>
      <ErrorMessage error={change.error} />

      <ResourceTabs projectId={projectId} resource={resource} />
      {tab !== 'overview' ? (
        <ResourceTabBody tab={tab} projectId={projectId} resource={resource} />
      ) : (
        <>
          <ProviderStats code={resource} database={isDatabase ? resource : null} lastCard={null} />

          <div className={styles.columns}>
            <div className={styles.column}>
              {!isDatabase && (
                <Section
                  title="Build and deploy"
                  action={
                    config && (
                      <Button
                        onClick={() => {
                          setEditing(true);
                        }}
                      >
                        Edit
                      </Button>
                    )
                  }
                >
                  <Rows
                    rows={[
                      ...(config?.repo
                        ? [['Source repo', config.repo] as [string, string]]
                        : config?.image
                          ? [['Source image', config.image] as [string, string]]
                          : []),
                      ['Root directory', config?.root_directory ?? '/'],
                      ['Build command', config?.build_command ?? `Detected by ${provider}`],
                      ['Start command', config?.start_command ?? `Detected by ${provider}`],
                      ['Healthcheck path', config?.healthcheck_path ?? 'None'],
                      [
                        'Replicas',
                        config?.replicas === null || config?.replicas === undefined
                          ? '1'
                          : String(config.replicas),
                      ],
                      ...(config?.region
                        ? [['Region', regionName(config.region)] as [string, string]]
                        : []),
                      ['Restart policy', restartWords(config?.restart_policy ?? null)],
                      ...(config?.cron_schedule
                        ? [['Cron schedule', config.cron_schedule] as [string, string]]
                        : []),
                      ...(details.data?.sleeps === null || details.data?.sleeps === undefined
                        ? []
                        : [['Serverless', details.data.sleeps ? 'On' : 'Off'] as [string, string]]),
                    ]}
                  />
                </Section>
              )}

              {isDatabase && connectVariables.length > 0 && (
                <Section title="Connect">
                  <Rows
                    rows={connectVariables.map((name): [string, ReactNode] => [
                      name.includes('PUBLIC') ? 'Public URL' : 'Private URL',
                      <CopyText key={name} text={`\${{${resource.name}.${name}}}`} />,
                    ])}
                  />
                  <p className={styles.frameNote}>
                    Add one of these as a variable on another service and {provider} fills in the
                    connection string. Hoista never reads it.
                  </p>
                </Section>
              )}

              <Section title="Reference variables">
                {uses.length === 0 && usedBy.length === 0 ? (
                  <p className={styles.empty}>
                    No service references it, and it references no other service.
                  </p>
                ) : (
                  <Rows
                    rows={[
                      ...uses.map((item): [string, ReactNode] => [
                        `Uses ${item.service}`,
                        <span key={item.variable} className="mono">
                          {item.variable}
                        </span>,
                      ]),
                      ...usedBy.map((item): [string, ReactNode] => [
                        `Used by ${item.resource.name}`,
                        <span key={item.resource.id + item.via} className="mono">
                          {item.via}
                        </span>,
                      ]),
                    ]}
                  />
                )}
              </Section>

              {(details.data?.volumes.length ?? 0) > 0 && (
                <Section title="Volumes">
                  <Rows
                    rows={(details.data?.volumes ?? []).map((volume): [string, ReactNode] => [
                      volume.name,
                      <span key={volume.id}>
                        <span className="mono">{volume.mount_path ?? 'not mounted'}</span>
                        {typeof volume.used_mb === 'number' && (
                          <span className={styles.muted}>
                            {` · ${formatMegabytes(volume.used_mb)}${
                              typeof volume.size_mb === 'number'
                                ? ` of ${formatMegabytes(volume.size_mb)}`
                                : ''
                            }`}
                          </span>
                        )}
                      </span>,
                    ])}
                  />
                </Section>
              )}
            </div>

            <div className={styles.column}>
              <Section
                title="Public networking"
                action={
                  <Link className={styles.more} to={`/projects/${projectId}/domains`}>
                    Manage
                  </Link>
                }
              >
                {publicCount === 0 ? (
                  <p className={styles.empty}>
                    None. Only services in this {provider} project can reach it.
                  </p>
                ) : (
                  <Rows
                    rows={[
                      ...(details.data?.domains ?? []).map((domain): [string, ReactNode] => [
                        domain.custom ? 'Custom domain' : `${provider} domain`,
                        <a
                          key={domain.name}
                          className={styles.link}
                          href={`https://${domain.name}`}
                          onClick={(event) => {
                            event.preventDefault();
                            void openExternal(`https://${domain.name}`);
                          }}
                        >
                          {domain.name} ↗
                        </a>,
                      ]),
                      ...(details.data?.tcp_proxies ?? []).map((proxy): [string, ReactNode] => [
                        'TCP proxy',
                        <CopyText
                          key={proxy.id}
                          text={`${proxy.domain}:${String(proxy.proxy_port)}`}
                          note={`to port ${String(proxy.application_port)}`}
                        />,
                      ]),
                    ]}
                  />
                )}
              </Section>

              {details.data?.private_domain && (
                <Section title="Private networking">
                  <Rows
                    rows={[
                      [
                        'Private domain',
                        <CopyText key="private" text={details.data.private_domain} />,
                      ],
                    ]}
                  />
                </Section>
              )}

              <Section
                title="Deployments"
                action={
                  <Link className={styles.more} to={`/projects/${projectId}/deploys`}>
                    All deployments
                  </Link>
                }
              >
                {(details.data?.deployments.length ?? 0) === 0 ? (
                  <p className={styles.empty}>No deploys yet.</p>
                ) : (
                  <ul className={styles.events}>
                    {(details.data?.deployments ?? []).slice(0, 6).map((deployment, index) => (
                      <li key={deployment.id} className={styles.event}>
                        <span
                          className={cx(
                            styles.dot,
                            deployment.state === 'failed'
                              ? styles.bad
                              : deployment.state === 'succeeded'
                                ? styles.good
                                : styles.neutral,
                          )}
                        />
                        <span className={styles.eventTitle}>
                          {commitUrl(deployment.repo, deployment.commit) ? (
                            <ExternalLink
                              variant="text"
                              className={styles.commitLink}
                              url={commitUrl(deployment.repo, deployment.commit) ?? ''}
                            >
                              <span className="mono">{(deployment.commit ?? '').slice(0, 7)}</span>
                            </ExternalLink>
                          ) : (
                            <span className="mono">
                              {(deployment.commit ?? deployment.id).slice(0, 7)}
                            </span>
                          )}
                          {deployment.message ? ` · ${deployment.message}` : ''}
                        </span>
                        <span className={styles.muted}>
                          {deploymentWord(deployment, index, details.data?.deployments ?? [])} ·{' '}
                          {timeAgo(deployment.created_at)}
                        </span>
                      </li>
                    ))}
                  </ul>
                )}
              </Section>

              <Section
                title="Variables"
                action={
                  <Link
                    className={styles.more}
                    to={`/projects/${projectId}/resources/${resource.id}/env`}
                  >
                    Manage
                  </Link>
                }
              >
                <p className={styles.empty}>
                  {details.data?.variable_names
                    ? `${String(details.data.variable_names.length)} set at ${provider}. Values stay there; Hoista never reads them.`
                    : `${provider} didn’t say.`}
                </p>
              </Section>

              <p className={styles.footnote}>
                Backups, environments, scaling by region, edge and more:{' '}
                <Link
                  to={`/projects/${projectId}/provider?resource=${encodeURIComponent(resource.id)}`}
                >
                  everything {provider} can do with {resource.name}
                </Link>
                .
              </p>
            </div>
          </div>
        </>
      )}

      {editing && config && (
        <ConfigureDialog
          resourceId={resource.id}
          name={resource.name}
          provider={provider}
          config={config}
          onClose={() => {
            setEditing(false);
          }}
        />
      )}
      {stopping && (
        <Dialog
          title={`Stop ${resource.name}?`}
          onClose={() => {
            setStopping(false);
          }}
          actions={
            <>
              <Button
                large
                onClick={() => {
                  setStopping(false);
                }}
              >
                Cancel
              </Button>
              <Button
                variant="danger"
                large
                busy={change.isPending}
                busyLabel="Stopping"
                onClick={() => {
                  change.mutate(
                    { resourceId: resource.id, body: { action: 'stop' } },
                    {
                      onSuccess: () => {
                        setStopping(false);
                      },
                    },
                  );
                }}
              >
                Stop it
              </Button>
            </>
          }
        >
          <p className={styles.dialogText}>
            {provider} stops the running version. It serves nothing until you redeploy or restart
            it.
          </p>
          <ErrorMessage error={change.error} />
        </Dialog>
      )}
    </>
  );
}

/** Railway's own names for its restart policies. */
function restartWords(policy: string | null): string {
  switch ((policy ?? '').toUpperCase()) {
    case 'ALWAYS':
      return 'Always';
    case 'NEVER':
      return 'Never';
    default:
      return 'On failure';
  }
}

/** Railway's word for a deployment: the newest that worked is Active. */
function deploymentWord(
  deployment: ProviderDetails['deployments'][number],
  index: number,
  all: ProviderDetails['deployments'],
): string {
  const status = (deployment.status ?? '').toUpperCase();
  if (status === 'SUCCESS') {
    const active = all.findIndex((item) => (item.status ?? '').toUpperCase() === 'SUCCESS');
    return active === index ? 'Active' : 'Success';
  }
  const words: Record<string, string> = {
    REMOVED: 'Removed',
    REMOVING: 'Removing',
    FAILED: 'Failed',
    CRASHED: 'Crashed',
    SLEEPING: 'Sleeping',
    SKIPPED: 'Skipped',
    BUILDING: 'Building',
    DEPLOYING: 'Deploying',
    INITIALIZING: 'Initializing',
    QUEUED: 'Queued',
    WAITING: 'Waiting',
    NEEDS_APPROVAL: 'Needs approval',
  };
  return words[status] ?? (deployment.state === 'failed' ? 'Failed' : 'Done');
}
