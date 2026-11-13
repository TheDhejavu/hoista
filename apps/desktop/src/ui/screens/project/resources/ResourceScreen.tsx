/**
 * One resource of a project (its website, database, file storage…): its
 * headline numbers, what uses it, recent events from the provider and its
 * settings there.
 */
import { useCan } from '@ui/api/providers';
import { Fragment, useState, type ReactNode } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { isResourceTab, ResourceTabBody, ResourceTabs, type ResourceTab } from './ResourceTabs';
import { RowMenu } from '@ui/components/RowMenu';
import { splitSource } from '@ui/lib/links';
import { AddressLine, SourceLine } from './SourceLine';
import { RemoveFromProject } from '../RemoveFromProject';
import type { ProviderTarget, ResourceDetail } from '@shared/api';
import { DeleteAtProvider } from '../DeleteAtProvider';
import { ChartStats, useCharted } from './ChartStats';
import { FactsSection, factsFor } from './ProviderFacts';
import { ProviderResourceView } from './ProviderResourceView';
import { Rows, Section } from './ResourceSection';
import { serviceName } from '@shared/providers';
import { useProject, useProjectSummary, useResource } from '@ui/api/projects';
import { openExternal } from '@ui/api/system';
import { Button } from '@ui/components/Button';
import { QueryState } from '@ui/components/QueryState';
import { SettingGroupSkeleton } from '@ui/components/SettingRow';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { statusLook } from '@ui/lib/status';
import { cx } from '@ui/lib/cx';
import { timeAgo } from '@ui/lib/format';
import { useRequiredParam } from '@ui/lib/useRequiredParam';
import { CrumbBar } from '../ProjectLayout';
import { resourceTerm, runsCode } from '../resourceKinds';
import styles from './ResourceScreen.module.css';

export function ResourceScreen() {
  const projectId = useRequiredParam('projectId');
  const resourceId = useRequiredParam('resourceId');
  const asked = useParams().tab;
  const tab: ResourceTab = isResourceTab(asked) ? asked : 'overview';
  const resource = useResource(resourceId);
  const summary = useProjectSummary(projectId);
  const projectName = summary?.name ?? 'this project';
  const watching = summary?.mode === 'watching';
  const can = useCan();

  return (
    <>
      <CrumbBar
        name={summary?.name ?? null}
        projectId={projectId}
        child={resource.data?.resource.name ?? null}
      />
      <div className={styles.page}>
        <QueryState query={resource} skeleton={<SettingGroupSkeleton cards={2} rows={3} />}>
          {(detail) =>
            // The deploy-and-run page is for providers that run code Hoista can redeploy.
            watching && can(detail.resource.provider, 'provider_deploys') ? (
              <ProviderResourceView
                projectId={projectId}
                projectName={projectName}
                detail={detail}
                tab={tab}
              />
            ) : (
              <ResourceContent
                projectId={projectId}
                projectName={projectName}
                detail={detail}
                tab={tab}
              />
            )
          }
        </QueryState>
      </div>
    </>
  );
}

function ResourceContent({
  projectId,
  projectName,
  detail,
  tab,
}: {
  projectId: string;
  projectName: string;
  detail: ResourceDetail;
  tab: ResourceTab;
}) {
  const { resource } = detail;
  const providerName = serviceName(resource.provider);
  const { label, tone } = statusLook(resource.status);
  const can = useCan();
  const project = useProject(projectId).data;
  const target: ProviderTarget = { kind: 'resource', id: resource.id };
  // The charts above already count requests over 24 hours; a second, daily
  // count of the same thing beside them reads as a contradiction.
  const charted = useCharted(resource);
  const stats = detail.stats.filter((stat) => !(charted && /^requests\b/i.test(stat.label)));
  const facts = factsFor(resource);
  const happened = facts.find((item) => item.events);
  const navigate = useNavigate();
  const [dialog, setDialog] = useState<'remove' | 'delete' | null>(null);
  const watching = project?.project.mode === 'watching';
  const last = (project?.resources.length ?? 1) <= 1;

  // Where the project's code is: its repository on GitHub, or its folder.
  const source = project?.project.source;
  const repo = source?.kind === 'github' ? splitSource(source.ref) : null;

  // Each section goes in the shorter column, so the two stay even.
  const sections: { key: string; weight: number; node: ReactNode }[] = [
    {
      key: 'used-by',
      weight: Math.max(detail.used_by.length, 1) + 2,
      node: (
        <Section title="Used by">
          {detail.used_by.length === 0 ? (
            <p className={styles.boxNote}>Nothing else in Hoista uses it.</p>
          ) : (
            <ul className={styles.events}>
              {detail.used_by.map((user) => (
                <li key={user.resource_id} className={styles.event}>
                  <ServiceIcon id={user.provider} size={14} />
                  <Link
                    className={styles.eventTitle}
                    to={`/projects/${projectId}/resources/${user.resource_id}`}
                  >
                    {user.name}
                  </Link>
                  <span className={cx('mono', styles.muted)}>{user.via}</span>
                </li>
              ))}
            </ul>
          )}
        </Section>
      ),
    },
    ...facts
      .filter((item) => !item.events)
      .map((item) => ({
        key: `${item.op}-${item.title}`,
        weight: (item.rows?.length ?? 6) + 2,
        node: <FactsSection facts={item} target={target} />,
      })),
    happened
      ? {
          key: 'events',
          weight: 10,
          node: <FactsSection facts={{ ...happened, title: 'Recent events' }} target={target} />,
        }
      : {
          key: 'events',
          weight: Math.max(detail.events.length, 1) + 2,
          node: (
            <Section title={`Recent events from ${providerName}`}>
              {detail.events.length === 0 ? (
                <p className={styles.boxNote}>Nothing has happened lately.</p>
              ) : (
                <ul className={styles.events}>
                  {detail.events.map((event) => (
                    <li key={`${event.at}-${event.title}`} className={styles.event}>
                      <span
                        className={cx(
                          styles.dot,
                          event.tone === 'good'
                            ? styles.good
                            : event.tone === 'bad'
                              ? styles.bad
                              : styles.neutral,
                        )}
                      />
                      <span className={styles.eventTitle} title={event.title}>
                        {event.title}
                      </span>
                      <span className={styles.muted}>{timeAgo(event.at)}</span>
                    </li>
                  ))}
                </ul>
              )}
            </Section>
          ),
        },
    ...(detail.settings.length > 0
      ? [
          {
            key: 'settings',
            weight: detail.settings.length + 2,
            node: (
              <Section title="Details">
                <Rows
                  rows={[
                    ...detail.settings.map((row): [string, ReactNode] => [row.label, row.value]),
                  ]}
                />
              </Section>
            ),
          },
        ]
      : []),
  ];
  const columns: { weight: number; nodes: ReactNode[] }[] = [
    { weight: 0, nodes: [] },
    { weight: 0, nodes: [] },
  ];
  for (const section of sections) {
    const [left, right] = columns;
    const column = left && right && right.weight < left.weight ? right : left;
    if (!column) continue;
    column.weight += section.weight;
    column.nodes.push(<Fragment key={section.key}>{section.node}</Fragment>);
  }

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
          <div className={cx('mono', styles.summary)} title={detail.status_note ?? undefined}>
            {[providerName, resourceTerm(resource), resource.region].filter(Boolean).join(' · ')}
          </div>
          <AddressLine url={resource.url} />
          {/* Where its code comes from; a database or bucket has none. */}
          {runsCode(resource.kind) && (
            <SourceLine
              repo={repo?.repo ?? source?.git?.repository}
              branch={source?.git?.branch}
              commit={repo?.commit ?? source?.git?.commit}
              folder={source?.kind === 'folder' ? source.ref : null}
            />
          )}
        </div>
        <div className={styles.actions}>
          <Button
            onClick={() => {
              void openExternal(detail.console_url);
            }}
          >
            Open in {providerName} ↗
          </Button>
          <RowMenu
            label={`More for ${resource.name}`}
            items={[
              ...(can(resource.provider, 'provider_operations')
                ? [
                    {
                      label: `Everything on ${providerName}`,
                      onSelect: () => {
                        void navigate(
                          `/projects/${projectId}/provider?resource=${encodeURIComponent(resource.id)}`,
                        );
                      },
                    },
                  ]
                : []),
              {
                label: 'Move to another project',
                onSelect: () => {
                  void navigate(`/projects/${projectId}/resources/${resource.id}/settings`);
                },
              },
              ...(watching
                ? [
                    {
                      label: 'Remove from project',
                      onSelect: () => {
                        setDialog('remove');
                      },
                    },
                  ]
                : []),
              ...(watching && can(resource.provider, 'delete')
                ? [
                    {
                      label: `Delete on ${providerName}`,
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
        {watching && (
          <>
            <RemoveFromProject
              resource={resource}
              projectName={projectName}
              last={last}
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
              last={last}
              opened={dialog === 'delete'}
              onClosed={() => {
                setDialog(null);
              }}
            />
          </>
        )}
      </div>

      <ResourceTabs projectId={projectId} resource={resource} />
      {tab !== 'overview' ? (
        <ResourceTabBody tab={tab} projectId={projectId} resource={resource} />
      ) : (
        <>
          <ChartStats resource={resource} />
          {stats.length > 0 && (
            <div className={styles.stats}>
              {stats.map((stat) => (
                <div key={stat.label} className={styles.card}>
                  <div className={styles.cardLabel}>{stat.label}</div>
                  <div className={styles.cardValue}>{stat.value}</div>
                  {stat.note && <div className={styles.cardNote}>{stat.note}</div>}
                </div>
              ))}
            </div>
          )}

          <div className={styles.columns}>
            {columns.map((column, index) => (
              <div key={index} className={styles.column}>
                {column.nodes}
              </div>
            ))}
          </div>
        </>
      )}
    </>
  );
}
