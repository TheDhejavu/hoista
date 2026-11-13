/**
 * The project's provider, at `/projects/:appId/provider`, laid out by the things that exist
 * there (environments, volumes and backups, networking, alerts…) rather
 * than by API call. Each section shows what the provider reports straight away
 * and puts what you can change beside it. A key says what each kind of
 * button does: Look changes nothing, Change runs at the provider now and shows
 * in Activity, Asks first can't be undone. Every raw operation is still
 * one click away, for anything the sections don't cover.
 */
import { runsCode } from '../resourceKinds';
import { useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { CircleHelp, Search } from 'lucide-react';
import type { ProviderOperation, ProviderQuickAction, ProviderTarget, Resource } from '@shared/api';
import { serviceName, providerEntry } from '@shared/providers';
import { useProject } from '@ui/api/projects';
import {
  useProviderOperations,
  useProviderQuickActions,
  useProviderRead,
} from '@ui/api/providerOperations';
import { SkeletonRows } from '@ui/components/Skeleton';
import { QueryState } from '@ui/components/QueryState';
import { SettingGroupSkeleton } from '@ui/components/SettingRow';
import { ProjectTab } from '../ProjectTab';
import { PillSelect } from '@ui/components/Select';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { cx } from '@ui/lib/cx';
import { useRequiredParam } from '@ui/lib/useRequiredParam';
import { FieldWordsContext } from './Result';
import { useCan, useProviderAbilities } from '@ui/api/providers';
import { sectionsFor, type Section } from './sections';
import styles from './ProviderTab.module.css';
import shared from './provider.module.css';
import { ResourceSelect } from '@ui/components/ResourceSelect';
import { Volumes } from './Volumes';
import { ActionBar, ActionButton, ResourceNameContext, type RunSpec } from './Runner';
import { OperationsExplorer } from './OperationsExplorer';
import { ViewCard } from './ViewCard';

// Each operation shows what it is in the provider's own terms (GET, DELETE, Query,
// Mutation, Command); its colour says whether it reads, changes, or can't be
// undone. These words are only for an operation that doesn't say.
export const RISK_LABEL = { read: 'Read', change: 'Write', destroy: 'Delete' } as const;

/** The provider's own words for operations of one risk, such as "POST · PATCH". */
function requestsOf(ops: ProviderOperation[], risk: ProviderOperation['risk']): string {
  const words = [...new Set(ops.filter((op) => op.risk === risk).map((op) => op.request))].filter(
    Boolean,
  );
  return words.length > 0 ? words.slice(0, 3).join(' · ') : RISK_LABEL[risk];
}

export function ProviderTab() {
  const projectId = useRequiredParam('projectId');
  const project = useProject(projectId);
  return (
    <QueryState
      query={project}
      skeleton={
        <ProjectTab>
          <SettingGroupSkeleton cards={2} rows={3} />
        </ProjectTab>
      }
    >
      {(detail) => <ProviderScreen projectId={projectId} resources={detail.resources} />}
    </QueryState>
  );
}

/**
 * What the buttons do, behind a "?" by the title: shown on hover or focus
 * instead of taking a row of the page.
 */
function Help({ provider, ops }: { provider: string; ops: ProviderOperation[] }) {
  return (
    <span className={styles.help}>
      <button type="button" className={styles.helpButton} aria-describedby="provider-help">
        <CircleHelp size={15} aria-hidden="true" />
        <span className={styles.srOnly}>What the buttons do</span>
      </button>
      <span id="provider-help" role="tooltip" className={styles.helpPop}>
        <span className={styles.helpTitle}>Operations by colour</span>
        <span className={shared.helpGrid}>
          <span className={shared.risk}>{requestsOf(ops, 'read')}</span>
          <span>Reads from the {provider} API. Nothing changes.</span>
          <span className={shared.risk}>{requestsOf(ops, 'change')}</span>
          <span>Changes it at {provider} straight away. Activity logs the exact request.</span>
          <span className={cx(shared.risk, shared.destroy)}>{requestsOf(ops, 'destroy')}</span>
          <span>Can’t be undone, so it needs your approval first. Its buttons are red.</span>
        </span>
        <span className={styles.helpFoot}>
          Your agent can run the same operations through Hoista’s MCP server.
        </span>
      </span>
    </span>
  );
}

function ProviderScreen({
  projectId,
  resources: all,
}: {
  projectId: string;
  resources: Resource[];
}) {
  // Providers whose details Hoista reads; resources elsewhere are listed but not yet open.
  const abilities = useProviderAbilities();
  const can = useCan();
  const covered = (resource: Resource): boolean =>
    can(resource.provider, 'provider_details') || can(resource.provider, 'provider_operations');
  const resources = all.filter(covered);
  const [search, setSearch] = useSearchParams();
  const asked = search.get('resource');
  const code =
    resources.find((resource) => resource.id === asked) ??
    resources.find((resource) => runsCode(resource.kind)) ??
    resources[0];
  const [resourceId, setResourceId] = useState<string | null>(code?.id ?? null);
  const resource = resources.find((item) => item.id === resourceId) ?? code;
  const target: ProviderTarget | null = resource ? { kind: 'resource', id: resource.id } : null;
  const ops = useProviderOperations(target);
  const quick = useProviderQuickActions(target);
  const [environment, setEnvironment] = useState<string | null>(null);
  const sectionId = search.get('section') ?? 'overview';
  const byName = useMemo(() => new Map((ops.data ?? []).map((op) => [op.name, op])), [ops.data]);
  // Environments are a Railway idea; other providers have no such read.
  const environments = useProviderRead<{ edges?: { node: { id: string; name: string } }[] }>(
    byName.has('environments') ? target : null,
    'environments',
  );
  // A section shows when it's for this kind of resource and Hoista can read
  // at least one of its views for it (a section of only buttons shows when
  // its buttons are there).
  const sections = sectionsFor(resource?.provider ?? '').filter(
    (section) =>
      (!section.refs ||
        (resource !== undefined &&
          section.refs.some((prefix) => resource.provider_ref.startsWith(prefix)))) &&
      (section.special !== undefined ||
        section.views.some((view) => aimed(byName.get(view.op))) ||
        (section.views.length === 0 && section.actions.some((action) => byName.has(action.op)))),
  );
  const section = sections.find((item) => item.id === sectionId) ?? sections[0];
  const quickLabels = new Set((quick.data ?? []).map((action) => action.label));
  const quickOps = new Set(
    (quick.data ?? []).filter((action) => byName.has(action.operation)).map((a) => a.operation),
  );

  if (abilities.isPending) return <SkeletonRows count={4} />;
  if (!resource || !target) {
    return (
      <p className={shared.empty}>
        This project has nothing on a provider that Hoista can reach yet.
      </p>
    );
  }
  const provider = serviceName(resource.provider);
  const envOptions = [
    { value: null as string | null, label: 'Same as the service' },
    ...(environments.data?.edges ?? []).map((edge) => ({
      value: edge.node.id,
      label: edge.node.name,
    })),
  ];
  const open = (id: string) => {
    setSearch(
      (current) => {
        const next = new URLSearchParams(current);
        next.set('section', id);
        return next;
      },
      { replace: true },
    );
  };

  return (
    <FieldWordsContext.Provider value={providerEntry(resource.provider)?.fieldWords}>
      <ResourceNameContext.Provider value={resource.name}>
        <div className={styles.page}>
          <header className={styles.head}>
            <div className={styles.title}>
              <span className={styles.logo}>
                <ServiceIcon id={resource.provider} size={16} />
              </span>
              <div>
                <h2 className={styles.titleLine}>
                  <span title={provider}>{resource.name}</span>
                  <Help provider={provider} ops={ops.data ?? []} />
                </h2>
                <p className={shared.lede}>
                  What {provider} reports for {resource.name}, and what you can change there.
                  Looking never changes anything.
                </p>
                <QuickActions actions={quick.data ?? []} byName={byName} target={target} />
              </div>
            </div>
            <div className={styles.controls}>
              {envOptions.length > 2 && (
                <PillSelect
                  label={`${provider} environment`}
                  value={environment}
                  options={envOptions}
                  onChange={setEnvironment}
                />
              )}
              {resources.length > 1 && (
                <ResourceSelect
                  resources={resources}
                  value={resource.id}
                  onChange={(id: string) => {
                    setResourceId(id);
                  }}
                />
              )}
            </div>
          </header>

          <QueryState query={ops} skeleton={<SkeletonRows count={5} columns={3} />}>
            {(list) => (
              <div className={styles.layout}>
                <nav className={styles.areas} aria-label="Sections">
                  <label className={styles.find}>
                    <Search size={13} aria-hidden="true" />
                    <input
                      type="search"
                      placeholder="Search operations"
                      aria-label={`Search ${provider}’s operations`}
                      value={search.get('q') ?? ''}
                      onChange={(event) => {
                        const q = event.target.value;
                        setSearch(
                          (current) => {
                            const next = new URLSearchParams(current);
                            if (q) {
                              next.set('q', q);
                              next.set('section', 'all');
                            } else next.delete('q');
                            return next;
                          },
                          { replace: true },
                        );
                      }}
                    />
                  </label>
                  {sections.map((item) => (
                    <button
                      key={item.id}
                      type="button"
                      className={cx(
                        styles.area,
                        sectionId !== 'all' && item.id === section?.id && styles.areaOn,
                      )}
                      onClick={() => {
                        open(item.id);
                      }}
                    >
                      <span>{item.label}</span>
                    </button>
                  ))}
                  <span className={styles.areaSep} />
                  <button
                    type="button"
                    className={cx(styles.area, sectionId === 'all' && styles.areaOn)}
                    onClick={() => {
                      open('all');
                    }}
                  >
                    <span>Every operation</span>
                    <span className={styles.count}>{list.length}</span>
                  </button>
                </nav>
                <div className={styles.main}>
                  {sectionId === 'all' || !section ? (
                    <OperationsExplorer
                      key={resource.id}
                      query={search.get('q') ?? ''}
                      setQuery={(q) => {
                        setSearch(
                          (current) => {
                            const next = new URLSearchParams(current);
                            if (q) next.set('q', q);
                            else next.delete('q');
                            return next;
                          },
                          { replace: true },
                        );
                      }}
                      ops={list}
                      target={target}
                      provider={serviceName(resource.provider)}
                      kind={resource.kind}
                    />
                  ) : (
                    <SectionView
                      key={`${section.id}-${resource.id}-${environment ?? ''}`}
                      projectId={projectId}
                      section={section}
                      byName={byName}
                      target={target}
                      resourceId={resource.id}
                      environment={environment}
                      quickOps={quickOps}
                      quickLabels={quickLabels}
                    />
                  )}
                </div>
              </div>
            )}
          </QueryState>
        </div>
      </ResourceNameContext.Provider>
    </FieldWordsContext.Provider>
  );
}

/**
 * The provider's common tasks for this resource, from its manifest: a row of
 * buttons above the sections. Each opens the operation's form with its
 * arguments set, and offers a dry run first where there is one.
 */
function QuickActions({
  actions,
  byName,
  target,
}: {
  actions: ProviderQuickAction[];
  byName: Map<string, ProviderOperation>;
  target: ProviderTarget;
}) {
  // Only things to do: what the provider reports already has a section of its
  // own on the left, so a button for it would be a second way to the same place.
  const shown = actions.flatMap((action) => {
    const op = byName.get(action.operation);
    return op && op.risk !== 'read' ? [{ action, op }] : [];
  });
  if (shown.length === 0) return null;
  return (
    <section className={styles.quick} aria-label="Common tasks">
      <div className={shared.actions}>
        {shown.map(({ action, op }) => (
          <ActionButton
            key={action.label}
            op={op}
            label={action.label}
            about={action.description}
            target={target}
            preset={action.args}
            tryFirst={action.try_first}
          />
        ))}
      </div>
    </section>
  );
}

/** One section: its actions on top, then everything the provider reports for it. */
function SectionView({
  projectId,
  section,
  byName,
  target,
  resourceId,
  environment,
  quickOps,
  quickLabels,
}: {
  projectId: string;
  section: Section;
  byName: Map<string, ProviderOperation>;
  target: ProviderTarget;
  resourceId: string;
  environment: string | null;
  /** Operations already in the header's common tasks, not repeated here. */
  quickOps: Set<string>;
  quickLabels: Set<string>;
}) {
  const env = (op: ProviderOperation | undefined): Record<string, unknown> =>
    environment && op?.args.some((arg) => arg.name === 'environmentId')
      ? { environmentId: environment }
      : {};
  const actions: RunSpec[] = section.actions.flatMap((action) => {
    const op = byName.get(action.op);
    return op && !quickOps.has(op.name) && !quickLabels.has(action.label)
      ? [
          {
            op,
            label: action.label,
            preset: { ...env(op), ...action.args },
            current: action.current,
          },
        ]
      : [];
  });
  const views = section.views.filter((view) => aimed(byName.get(view.op)));

  return (
    <div className={shared.section}>
      <div className={shared.sectionHead}>
        <div>
          <h3>{section.label}</h3>
          {section.about && <p className={shared.lede}>{section.about}</p>}
        </div>
        <ActionBar actions={actions} target={target} />
      </div>
      {section.link && (
        <Link className={styles.sectionLink} to={`/projects/${projectId}/${section.link.tab}`}>
          {section.link.label} →
        </Link>
      )}
      {section.special === 'volumes' && (
        <Volumes resourceId={resourceId} target={target} byName={byName} />
      )}
      {views.map((view) => (
        <ViewCard
          key={view.op}
          view={view}
          target={target}
          byName={byName}
          args={{ ...view.args, ...env(byName.get(view.op)) }}
        />
      ))}
    </div>
  );
}

/**
 * Whether Hoista can read an operation for this resource without asking:
 * every argument it needs is filled in from the resource. A Pages read on a
 * Worker isn't, so it isn't shown there.
 */
function aimed(op: ProviderOperation | undefined): op is ProviderOperation {
  return op !== undefined && op.args.every((arg) => !arg.required || arg.filled_in !== null);
}
