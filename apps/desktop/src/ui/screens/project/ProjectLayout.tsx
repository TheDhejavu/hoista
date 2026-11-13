/**
 * The frame around every project screen: a breadcrumb bar (Projects / name), the project's picture, name, status, address,
 * version and providers, "Ask your agent" and "Open site", then the section
 * tabs. The current tab renders below. Resource pages (`resources/:resourceId`) have
 * their own header, so they render alone.
 */
import { providerEntry, serviceName } from '@shared/providers';
import { useEffect, useRef, useState } from 'react';
import { Link, NavLink, Outlet, useLocation, useMatch, useNavigate } from 'react-router-dom';
import { EngineError } from '@shared/engine';
import { showToast } from '@ui/lib/toast';
import type { ProjectSummary, Resource } from '@shared/api';
import { useDomains, useProject, useProjectSummary, useSecrets } from '@ui/api/projects';
import { useDeploys } from '@ui/api/deploys';
import { openExternal } from '@ui/api/system';
import { ProjectShot } from '@ui/components/ProjectShot';
import { AskAgentDialog, type SuggestedPrompt } from '@ui/components/AskAgentDialog';
import { Button } from '@ui/components/Button';
import { QueryState } from '@ui/components/QueryState';
import { Skeleton } from '@ui/components/Skeleton';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { statusLook } from '@ui/lib/status';
import { cx } from '@ui/lib/cx';
import { hostOf } from '@ui/lib/format';
import { useRequiredParam } from '@ui/lib/useRequiredParam';
import { previousVersion } from './deploys/describe';
import { useCan } from '@ui/api/providers';
import { useCurrentEnvironment } from '@ui/api/environments';
import { runsCode } from './resourceKinds';
import { CommandButton } from '@ui/components/CommandButton';
import { useHoistCommand } from '@ui/lib/hoistCommand';
import { projectLook } from '@ui/lib/projectLook';
import styles from './ProjectLayout.module.css';

const TABS = [
  { path: '', label: 'Overview' },
  { path: 'map', label: 'Map' },
  { path: 'deploys', label: 'Deploys' },
  { path: 'logs', label: 'Logs' },
  { path: 'env', label: 'Variables' },
  { path: 'domains', label: 'Domains' },
  { path: 'settings', label: 'Settings' },
];
// Activity is reached from Overview's "See all activity", not a tab of its own.

/** Tabs a project reopens on: every tab but Overview, which is where it opens anyway. */
const REOPENS_ON = new Set([
  'map',
  'deploys',
  'logs',
  'metrics',
  'env',
  'domains',
  'settings',
  'provider',
]);
const LAST_TAB = 'hoista.lastTab.';

/** The `hoist` command that shows what each tab shows, for a project named `name`. */
function tabCommand(tab: string, name: string): string[] {
  switch (tab) {
    case 'map':
      return ['map', name];
    case 'deploys':
      return ['deploys', name];
    case 'logs':
      return ['logs', name, '--follow'];
    case 'metrics':
      return ['metrics', '--project', name];
    case 'env':
      return ['env', 'list', name];
    case 'domains':
      return ['domains', name];
    case 'provider':
    case 'settings':
      return ['resources', name];
    case 'activity':
      return ['activity', '--project', name];
    default:
      return ['status', name];
  }
}

function lastTab(projectId: string): string | null {
  try {
    const tab = window.localStorage.getItem(LAST_TAB + projectId);
    return tab && REOPENS_ON.has(tab) ? tab : null;
  } catch {
    return null;
  }
}

function rememberTab(projectId: string, tab: string): void {
  try {
    if (REOPENS_ON.has(tab)) window.localStorage.setItem(LAST_TAB + projectId, tab);
    else window.localStorage.removeItem(LAST_TAB + projectId);
  } catch {
    // Not remembered this time; the project opens on Overview next time.
  }
}

export function ProjectLayout() {
  const projectId = useRequiredParam('projectId');
  const project = useProject(projectId);
  const summary = useProjectSummary(projectId);
  const onResource = useMatch('/projects/:projectId/resources/:resourceId');
  const onResourceTab = useMatch('/projects/:projectId/resources/:resourceId/:tab');
  const isResourcePage = onResource !== null || onResourceTab !== null;
  const { pathname } = useLocation();
  const navigate = useNavigate();
  // A project that's gone (deleted here, by an agent or at its provider) sends
  // you back to Projects instead of leaving a page that can't load.
  const gone = project.error instanceof EngineError && project.error.code === 'resource.not_found';
  useEffect(() => {
    if (!gone) return;
    showToast('That project no longer exists');
    void navigate('/', { replace: true });
  }, [gone, navigate]);

  // Opening a project from another environment (search, a link, a
  // notification) switches the window to that environment, so the sidebar,
  // Deploys and Activity all match the project you're looking at.
  const environment = useCurrentEnvironment();
  const home = project.data?.project.environment_id;
  useEffect(() => {
    if (!home || environment.environments.length === 0 || home === environment.id) return;
    const name = environment.environments.find((item) => item.id === home)?.name;
    if (!name) return;
    environment.set(home);
    showToast(`Showing ${name}`);
  }, [home, environment]);

  // Opening a project goes back to the tab you left it on. Choosing Overview
  // while you're in the project stays on Overview.
  const tab = pathname.split('/')[3] ?? '';
  const openedFor = useRef<string | null>(null);
  useEffect(() => {
    if (openedFor.current === projectId) return;
    openedFor.current = projectId;
    const last = tab === '' ? lastTab(projectId) : null;
    if (last) void navigate(`/projects/${projectId}/${last}`, { replace: true });
  }, [projectId, tab, navigate]);
  useEffect(() => {
    if (openedFor.current === projectId && !isResourcePage && tab !== 'activity') {
      rememberTab(projectId, tab);
    }
  }, [projectId, tab, isResourcePage]);

  if (isResourcePage) return <Outlet />;

  return (
    <>
      <CrumbBar name={summary?.name ?? null} />
      <div className={styles.header}>
        {summary ? (
          <ProjectTitle project={summary} resources={project.data?.resources ?? []} />
        ) : (
          <QueryState query={project} skeleton={<TitleSkeleton />}>
            {(data) => <ProjectTitle project={data.project} resources={data.resources} />}
          </QueryState>
        )}
      </div>
      <ProjectTabs projectId={projectId} />
      {/* Each tab fades in; the header above stays put. */}
      <div key={pathname} className={styles.tabContent}>
        <Outlet />
      </div>
    </>
  );
}

/**
 * Projects / name, or Projects / name / resource on a resource's page. A name
 * not known yet is a short placeholder, never an id.
 */
export function CrumbBar({
  name,
  projectId,
  child,
}: {
  name: string | null;
  /** With `child`: the project's id, so its name links back to it. */
  projectId?: string;
  /** The resource's name; null while it loads. */
  child?: string | null;
}) {
  const nameShown = name ?? <Skeleton width={72} later />;
  return (
    <div className={styles.crumbs}>
      <nav aria-label="Breadcrumb" className={styles.trail}>
        <Link to="/">Projects</Link>
        <span aria-hidden="true">/</span>
        {child !== undefined && projectId ? (
          <>
            <Link to={`/projects/${projectId}`}>{nameShown}</Link>
            <span aria-hidden="true">/</span>
            <span className={styles.here} aria-current="page">
              {child ?? <Skeleton width={96} later />}
            </span>
          </>
        ) : (
          <span className={styles.here} aria-current="page">
            {nameShown}
          </span>
        )}
      </nav>
    </div>
  );
}

/** Provider logos shown beside the name; the rest fold into +N. A project can use 15 providers. */
const PROVIDERS_SHOWN = 6;

/** The header's shape while the project loads: picture, name and status, one line below. */
function TitleSkeleton() {
  return (
    <div className={styles.title}>
      <span className={styles.picture}>
        <Skeleton width={88} height={56} />
      </span>
      <div className={styles.names}>
        <div className={cx(styles.nameRow, styles.nameRowSkeleton)}>
          <Skeleton width={160} height={28} />
          <Skeleton width={56} height={20} />
        </div>
        <div className={cx(styles.sub, styles.subSkeleton)}>
          <Skeleton width={240} height={12} />
        </div>
      </div>
    </div>
  );
}

/** `resources` is empty until the project's details arrive; only service names use it. */
function ProjectTitle({ project, resources }: { project: ProjectSummary; resources: Resource[] }) {
  const deploys = useDeploys(project.id);
  const items = deploys.data?.items ?? [];
  const previous = previousVersion(items)?.version ?? null;
  // What each service runs now: one version, or one per service when the
  // project has several, each named.
  const live = items.filter((deploy) => deploy.current);
  const byService = new Map<string, string>();
  for (const deploy of live) {
    const key = deploy.resource_id ?? '';
    if (!byService.has(key)) byService.set(key, deploy.version);
  }
  const versions = [...byService.entries()].map(([id, version]) => ({
    version,
    name: resources.find((resource) => resource.id === id)?.name ?? null,
  }));
  const { label, tone } = statusLook(project.status, project.idle_days);
  const { url } = project;

  return (
    <div className={styles.title}>
      <span className={styles.picture}>
        <ProjectPicture project={project} />
      </span>
      <div className={styles.names}>
        <div className={styles.nameRow}>
          <h1>{project.name}</h1>
          <span className={cx(styles.pill, project.status === 'failing' && styles.pillBad)}>
            <span className={cx(styles.dot, styles[tone])} />
            {label}
          </span>
        </div>
        <div className={styles.sub}>
          <span className={cx('mono', styles.address)} title={url ? hostOf(url) : undefined}>
            {url ? hostOf(url) : 'No domain'}
          </span>
          {versions.length === 1 && versions[0] && (
            <>
              <span aria-hidden="true">·</span>
              <span className="mono">{versions[0].version}</span>
            </>
          )}
          {versions.length > 1 && (
            <>
              <span aria-hidden="true">·</span>
              {/* One line however many services: each one's version is on hover and on Deploys. */}
              <span
                className={styles.versions}
                title={versions
                  .map((item) => `${item.name ?? 'Service'} ${item.version}`)
                  .join('\n')}
              >
                {versions.length} services deployed
              </span>
            </>
          )}
          <span aria-hidden="true">·</span>
          <span className={styles.providers}>
            {project.providers.slice(0, PROVIDERS_SHOWN).map((provider) => (
              <span key={provider} className={styles.providerTile} title={serviceName(provider)}>
                <ServiceIcon id={provider} size={11} />
              </span>
            ))}
            {project.providers.length > PROVIDERS_SHOWN && (
              <span
                className={cx(styles.providerTile, styles.providerMore)}
                title={project.providers.slice(PROVIDERS_SHOWN).map(serviceName).join(', ')}
              >
                +{project.providers.length - PROVIDERS_SHOWN}
              </span>
            )}
          </span>
        </div>
      </div>
      <div className={styles.actions}>
        <AskAgentButton
          projectName={project.name}
          previous={previous}
          redeploy={providerEntry(project.providers[0])?.words?.redeploy}
        />
        {url && (
          <Button
            variant="accent"
            large
            onClick={() => {
              void openExternal(url);
            }}
          >
            Open site ↗
          </Button>
        )}
      </div>
    </div>
  );
}

/**
 * The project's picture: a drawing of its page, or, for an API or scheduled
 * job that has no page, the logo of the provider it runs on.
 */
function ProjectPicture({ project }: { project: ProjectSummary }) {
  const kind = projectLook(project).kind;
  const [provider] = project.providers;
  if ((kind === 'api' || kind === 'cron') && provider) {
    return (
      <span className={styles.providerPicture} title={`Runs on ${serviceName(provider)}`}>
        <ServiceIcon id={provider} size={26} />
      </span>
    );
  }
  return <ProjectShot project={project} height={56} />;
}

/** Opens the prompts people usually want for this project: redeploy, roll back, find errors. */
function AskAgentButton({
  projectName,
  previous,
  redeploy,
}: {
  projectName: string;
  previous: string | null;
  /** What a redeploy does at the project's provider, in its words. */
  redeploy: string | undefined;
}) {
  const [isOpen, setOpen] = useState(false);
  const prompts: SuggestedPrompt[] = [
    {
      prompt: `Redeploy ${projectName} with Hoista`,
      hint: redeploy ? `${redeploy}.` : 'Deploys it again at its provider.',
    },
    ...(previous
      ? [
          {
            prompt: `Roll ${projectName} back to ${previous} with Hoista`,
            hint: 'Puts the previous version live again.',
          },
        ]
      : []),
    {
      prompt: `Check ${projectName}'s logs in Hoista and fix any errors`,
      hint: 'Your agent reads the same logs you see here.',
    },
  ];

  return (
    <>
      <Button
        large
        onClick={() => {
          setOpen(true);
        }}
      >
        Ask your agent
      </Button>
      {isOpen && (
        <AskAgentDialog
          title={`Ask your agent about ${projectName}`}
          prompts={prompts}
          onClose={() => {
            setOpen(false);
          }}
        />
      )}
    </>
  );
}

function ProjectTabs({ projectId }: { projectId: string }) {
  const detail = useProject(projectId).data;
  const resources = detail?.resources ?? [];
  const can = useCan();
  // All activity belongs to Overview.
  const onActivity = useMatch('/projects/:projectId/activity') !== null;
  // Resources on providers whose details or operations Hoista reaches get a
  // tab named after the provider (or "Providers" when there are several); its
  // dropdown picks which resource, on whichever provider, it shows.
  const reached = [
    ...new Set(
      resources
        .filter(
          (resource) =>
            can(resource.provider, 'provider_details') ||
            can(resource.provider, 'provider_operations'),
        )
        .map((resource) => resource.provider),
    ),
  ];
  const [onlyProvider] = reached;
  const providerTab =
    reached.length > 1 ? 'Providers' : onlyProvider ? serviceName(onlyProvider) : null;
  // A provider that reports numbers over time gets a Metrics tab after Logs,
  // for the resources it reports them for.
  const metrics = resources.some(
    (resource) =>
      can(resource.provider, 'metrics') &&
      (providerEntry(resource.provider)?.metricsRefs?.some((prefix) =>
        resource.provider_ref.startsWith(prefix),
      ) ??
        true),
  );
  // Only what the project's resources have: a database alone has no
  // deploys, logs, variables or domains to show. Hoista's own projects
  // deploy, so they have all of them.
  const managed = detail?.project.mode === 'managed';
  const code = managed || resources.some((resource) => runsCode(resource.kind));
  // A tab still shows whenever the project has something in it, so nothing
  // it holds is ever out of reach, and the tab open now stays.
  const deploys = (useDeploys(projectId).data?.items.length ?? 0) > 0;
  const variables = (useSecrets(projectId).data?.items.length ?? 0) > 0;
  const domains = (useDomains(projectId).data?.items.length ?? 0) > 0;
  const open = useLocation().pathname.split('/')[3];
  const shown: Record<string, boolean> = {
    map: resources.length > 1,
    deploys: code || deploys,
    logs: code,
    env: code || variables,
    domains: code || domains || resources.some((resource) => resource.kind === 'domain'),
  };
  const kept = (tab: { path: string }) => tab.path === open || (shown[tab.path] ?? true);
  const logs = TABS.findIndex((tab) => tab.path === 'logs') + 1;
  const tabs = [
    ...TABS.slice(0, logs).filter(kept),
    ...(metrics ? [{ path: 'metrics', label: 'Metrics' }] : []),
    ...TABS.slice(logs, -1).filter(kept),
    ...(providerTab ? [{ path: 'provider', label: providerTab }] : []),
    ...TABS.slice(-1),
  ];
  const command = useHoistCommand();
  const name = detail?.project.name;
  return (
    <nav aria-label="Project sections" className={styles.tabs}>
      {tabs.map((tab) => (
        <NavLink
          key={tab.label}
          to={tab.path ? `/projects/${projectId}/${tab.path}` : `/projects/${projectId}`}
          end={tab.path === ''}
          className={({ isActive }) =>
            cx(styles.tab, (isActive || (tab.path === '' && onActivity)) && styles.activeTab)
          }
        >
          {tab.label}
        </NavLink>
      ))}
      {name && (
        <span className={styles.tabCommand}>
          <CommandButton command={command(tabCommand(open ?? '', name))} />
        </span>
      )}
    </nav>
  );
}
