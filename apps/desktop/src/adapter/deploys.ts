/**
 * Deploys from the daemon: one deploy, a project's history, the latest across
 * projects, cancel and roll back.
 *
 * The visible version is the release number (`v3`); failed attempts have
 * none, so they show their attempt number instead (`attempt 7`). A deploy is
 * current when it is its project's newest live attempt; every list here is
 * newest-first and includes all newer attempts, so that is exact.
 */
import { agentDisplayName } from '@shared/agentLogos';
import type {
  Agent as DaemonAgent,
  ProjectOverview,
  Deploy as DaemonDeploy,
  DeployStep as DaemonStep,
  LogEntry,
  Page as DaemonPage,
  Source,
} from '@engine-api';
import type { Deploy, DeployLogLine, DeployStep, DeploySummary, Page, StepName } from '@shared/api';
import { SERVICES } from '@shared/providers';
import { RouteError, type Router } from '@shared/router';
import { actorOf } from './projects';
import { query, type Call } from './call';
import { providerDeploys, type ProviderReads } from './provider';
import { parseProviderDeploy } from '@shared/deployIds';

const STEPS: readonly StepName[] = [
  'detect',
  'plan',
  'secrets',
  'leak_check',
  'build',
  'release',
  'verify',
];

/** Resources that run code, and so are deployed. */
const CODE_KINDS: readonly string[] = ['web', 'service', 'job'];

export function addDeployRoutes(router: Router, call: Call, reads: ProviderReads): void {
  async function agentNames(): Promise<Map<string, string>> {
    const agents = await call<DaemonPage<DaemonAgent>>('GET', '/agents');
    return new Map(agents.items.map((agent) => [agent.id, agentDisplayName(agent.name)]));
  }

  async function one(id: string): Promise<Deploy> {
    const deploy = await call<DaemonDeploy>('GET', `/deploys/${encodeURIComponent(id)}`);
    const [history, names] = await Promise.all([
      call<DaemonPage<DaemonDeploy>>(
        'GET',
        `/projects/${encodeURIComponent(deploy.project_id)}/deploys`,
      ),
      agentNames(),
    ]);
    return toDeploy(deploy, currentIds(history.items).has(deploy.id), names);
  }

  router.on('GET', '/deploys', async (): Promise<Page<Deploy>> => {
    const [recent, names, projects] = await Promise.all([
      call<DaemonPage<DaemonDeploy>>('GET', '/deploys'),
      agentNames(),
      call<DaemonPage<ProjectOverview>>('GET', '/projects/summaries'),
    ]);
    const page = toPage(recent, names);
    // Watched projects deploy on their provider, and so do pieces an agent added
    // at a provider; add each provider's own record.
    const withProviders = projects.items
      .filter(
        (project) =>
          project.project.mode === 'watching' ||
          project.resources.some((resource) => CODE_KINDS.includes(resource.kind)),
      )
      .slice(0, 25);
    const fromProviders = (
      await Promise.all(
        withProviders.map(async (project) =>
          (await reads.providerDeployed(project.project.id)).flatMap(({ resource, details }) =>
            providerDeploys(project.project.id, resource, details),
          ),
        ),
      )
    ).flat();
    return {
      items: [...page.items, ...fromProviders]
        .sort((a, b) => b.created_at.localeCompare(a.created_at))
        .slice(0, 50),
      cursor: null,
    };
  });

  router.on('GET', '/deploys/:id', async ({ params }): Promise<Deploy> => {
    const id = params.id ?? '';
    const provider = parseProviderDeploy(id);
    if (!provider) return one(id);
    const { resourceId } = provider;
    const resource = await call<{ resource: Parameters<typeof providerDeploys>[1] }>(
      'GET',
      `/resources/${encodeURIComponent(resourceId)}/details`,
    );
    const found = providerDeploys(
      resource.resource.project_id ?? '',
      resource.resource,
      await reads.forResource(resourceId),
    ).find((deploy) => deploy.id === id);
    if (!found)
      throw new RouteError('deploy.not_found', 'The provider no longer lists this deployment.');
    return found;
  });

  router.on(
    'GET',
    '/projects/:id/deploys',
    async ({ params, query: search }): Promise<Page<Deploy>> => {
      const projectId = params.id ?? '';
      const cursor = search.get('cursor');
      const [history, names, watched] = await Promise.all([
        call<DaemonPage<DaemonDeploy>>(
          'GET',
          `/projects/${encodeURIComponent(projectId)}/deploys${query({ cursor })}`,
        ),
        agentNames(),
        // Provider history is one page; only add it to the first.
        cursor ? Promise.resolve([]) : reads.providerDeployed(projectId),
      ]);
      const page = toPage(history, names);
      const fromProviders = watched.flatMap(({ resource, details }) =>
        providerDeploys(projectId, resource, details),
      );
      if (fromProviders.length === 0) return page;
      return {
        items: [...page.items, ...fromProviders].sort((a, b) =>
          b.created_at.localeCompare(a.created_at),
        ),
        cursor: page.cursor,
      };
    },
  );

  // The terminal on a deploy's page. A provider deploy shows the provider's build and
  // runtime log; Hoista's own deploys show each step as it was recorded.
  router.on('GET', '/deploys/:id/log', async ({ params }): Promise<{ lines: DeployLogLine[] }> => {
    const id = params.id ?? '';
    const provider = parseProviderDeploy(id);
    if (provider) {
      const { resourceId, deploymentId } = provider;
      const entries = await call<LogEntry[]>(
        'GET',
        `/resources/${encodeURIComponent(resourceId)}/provider/deployments/${encodeURIComponent(deploymentId)}/logs`,
      );
      // Oldest first, timed from the first line, whatever order the provider sends.
      const at = (entry: LogEntry) => Date.parse(entry.timestamp ?? entry.observed_at);
      const sorted = [...entries].sort((a, b) => (at(a) || 0) - (at(b) || 0));
      const start = Math.min(...sorted.map(at).filter((ms) => !Number.isNaN(ms)));
      return {
        lines: sorted.map((entry) => ({
          offset_ms: Math.max(0, at(entry) - start) || 0,
          level: entry.level === 'error' ? 'error' : entry.level === 'warn' ? 'warn' : 'info',
          text: entry.source === 'build' ? `build · ${entry.message}` : entry.message,
        })),
      };
    }
    const deploy = await call<DaemonDeploy>('GET', `/deploys/${encodeURIComponent(id)}`);
    return {
      lines: deploy.steps.map((step) => ({
        offset_ms: 0,
        level: step.status === 'failed' ? 'error' : step.status === 'succeeded' ? 'ok' : 'info',
        text: `${step.name} · ${step.status}${step.message ? ` · ${step.message}` : ''}`,
      })),
    };
  });

  // Where a provider deploy went. Hoista's own deploys keep their plan in the engine.
  router.on('GET', '/deploys/:id/summary', async ({ params }): Promise<DeploySummary> => {
    const id = params.id ?? '';
    const made = parseProviderDeploy(id);
    if (!made) {
      throw new RouteError('deploy.summary_unavailable', 'This deploy has no summary to show yet.');
    }
    const { resourceId } = made;
    const { resource } = await call<{ resource: Parameters<typeof providerDeploys>[1] }>(
      'GET',
      `/resources/${encodeURIComponent(resourceId)}/details`,
    );
    const provider = SERVICES[resource.provider].name;
    return {
      runs_on: [provider],
      targets: [
        { label: 'Resource', value: resource.name },
        ...(resource.url ? [{ label: 'Address', value: resource.url }] : []),
      ],
    };
  });

  // The daemon acknowledges cancellation; the screen wants the deploy back.
  router.on('POST', '/deploys/:id/cancel', async ({ params }): Promise<Deploy> => {
    await call('POST', `/deploys/${encodeURIComponent(params.id ?? '')}/cancel`);
    return one(params.id ?? '');
  });

  router.on('POST', '/deploys/:id/rollback', async ({ params, body }) => {
    const id = params.id ?? '';
    // A provider deployment rolls back at the provider: it serves that deployment again.
    const provider = parseProviderDeploy(id);
    if (provider) {
      const { resourceId, deploymentId } = provider;
      await call('POST', `/resources/${encodeURIComponent(resourceId)}/provider/actions`, {
        action: 'rollback',
        deployment_id: deploymentId,
      });
      reads.forget(resourceId);
      return { deploy_id: id };
    }
    const started = await call<DaemonDeploy>(
      'POST',
      `/deploys/${encodeURIComponent(params.id ?? '')}/rollback`,
      body,
    );
    return { deploy_id: started.id };
  });
}

function toPage(page: DaemonPage<DaemonDeploy>, names: ReadonlyMap<string, string>): Page<Deploy> {
  const current = currentIds(page.items);
  return {
    items: page.items.map((deploy) => toDeploy(deploy, current.has(deploy.id), names)),
    cursor: page.next_cursor,
  };
}

/** The newest live attempt of each project in a newest-first list. */
export function currentIds(newestFirst: readonly DaemonDeploy[]): Set<string> {
  const seen = new Set<string>();
  const current = new Set<string>();
  for (const deploy of newestFirst) {
    if (deploy.status !== 'live' || seen.has(deploy.project_id)) continue;
    seen.add(deploy.project_id);
    current.add(deploy.id);
  }
  return current;
}

export function toDeploy(
  deploy: DaemonDeploy,
  current: boolean,
  agentNames: ReadonlyMap<string, string>,
): Deploy {
  const cancelled = deploy.failure?.code === 'deploy.cancelled';
  const failedStep = deploy.failure?.steps.find((name): name is StepName =>
    STEPS.includes(name as StepName),
  );
  return {
    id: deploy.id,
    project_id: deploy.project_id,
    version:
      deploy.release_version === null
        ? `attempt ${String(deploy.version)}`
        : `v${String(deploy.release_version)}`,
    source: sourceOf(deploy.source, deploy.title),
    started_by: actorOf(deploy, agentNames),
    status: cancelled ? 'cancelled' : deploy.status,
    steps: deploy.steps.flatMap(toStep),
    url: deploy.url,
    created_at: deploy.started_at,
    duration_ms: deploy.duration_ms,
    current,
    failure:
      deploy.failure && !cancelled
        ? {
            step: failedStep ?? null,
            message: deploy.failure.message,
            fix: null,
            missing_secrets: deploy.failure.missing_secrets,
          }
        : null,
  };
}

/** Pipeline steps only; bookkeeping entries such as `automatic_rollback` are left out. */
function toStep(step: DaemonStep): DeployStep[] {
  if (!STEPS.includes(step.name as StepName)) return [];
  return [
    {
      name: step.name as StepName,
      status: step.status === 'succeeded' ? 'done' : step.status,
      detail: step.message,
      duration_ms: null,
    },
  ];
}

function sourceOf(source: Source, title: string | null): Deploy['source'] {
  switch (source.kind) {
    case 'folder':
      return { kind: 'folder', ref: source.path, message: title };
    case 'github':
      return {
        kind: 'commit',
        ref: `${source.repository}@${source.commit.slice(0, 7)}`,
        message: title,
        repo: source.repository,
        commit: source.commit,
      };
    case 'unlinked':
      return { kind: 'folder', ref: '', message: title };
  }
}
