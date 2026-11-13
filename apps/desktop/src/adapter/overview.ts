/**
 * The project overview (`GET /projects/{id}`) from the daemon's `AppOverview`.
 *
 * Measured values pass through with their gaps. Fields the engine does not
 * measure yet (week-on-week change, last-24-hour errors, the top error, a
 * suggested question for your agent, per-resource usage) are null, so
 * the screen hides them instead of showing invented numbers.
 */
import { providerEntry } from '@shared/providers';
import { agentDisplayName } from '@shared/agentLogos';
import type {
  Agent as DaemonAgent,
  ProjectOverview,
  DeletionPreview,
  Page as DaemonPage,
  Resource as DaemonResource,
  Source,
  Usage,
} from '@engine-api';
import type { ProviderAbilities, ProjectDetail, Resource } from '@shared/api';
import { RouteError, type Router } from '@shared/router';
import { providerWeek, toProjectListItem } from './projects';
import type { Call } from './call';
import { lastProviderDeploy, laterDeploy, type ProviderReads } from './provider';
import { fromProviders } from './abilities';

export function addOverviewRoutes(router: Router, call: Call, reads: ProviderReads): void {
  // Projects already offered their provider's repository this run, linked or not.
  const triedLinking = new Set<string>();
  // Deleting a project: the engine wants the reviewed impact (its preview's
  // fingerprint and the exact name), so a change since the review stops it.
  // The person already typed the name in the confirm dialog.
  router.on('DELETE', '/projects/:id', async ({ params, body }): Promise<null> => {
    const id = encodeURIComponent(params.id ?? '');
    const input = (body ?? {}) as { backup?: unknown };
    const overview = await call<ProjectOverview>('GET', `/projects/${id}`);
    // What runs on its provider (watched) is deleted there, each resource with
    // its provider's own delete; a project left empty leaves Hoista with it.
    const watched = overview.resources.filter((resource) => resource.mode === 'watching');
    if (watched.length > 0) {
      const abilities = fromProviders(await call<ProviderAbilities>('GET', '/providers'));
      const stuck = watched.filter((resource) => !abilities.can(resource.provider, 'delete'));
      if (stuck.length > 0) {
        throw new RouteError(
          'app.delete_blocked',
          `Hoista can't delete ${stuck.map((resource) => resource.name).join(', ')} at ${providerEntry(stuck[0]?.provider ?? '')?.name ?? 'its provider'}. Delete ${stuck.length === 1 ? 'it' : 'them'} there, then try again.`,
        );
      }
      for (const resource of watched) {
        await call('POST', `/resources/${encodeURIComponent(resource.id)}/provider/actions`, {
          action: 'remove_service',
          confirmation_name: resource.name,
        });
      }
    }
    const left = await call<ProjectOverview>('GET', `/projects/${id}`).catch(() => null);
    if (!left) return null;
    if (left.project.mode === 'watching') {
      await call('POST', `/projects/${id}/stop-watching`);
      return null;
    }
    const preview = await call<DeletionPreview>('GET', `/projects/${id}/deletion-preview`);
    if (preview.blockers.length > 0) {
      throw new RouteError('app.delete_blocked', preview.blockers.join(' '));
    }
    await call('DELETE', `/projects/${id}`, {
      backup_databases: input.backup !== false,
      confirmation_name: preview.confirmation_name,
      fingerprint: preview.fingerprint,
    });
    return null;
  });

  router.on('GET', '/projects/:id', async ({ params }): Promise<ProjectDetail> => {
    const id = encodeURIComponent(params.id ?? '');
    const [overview, agents] = await Promise.all([
      call<ProjectOverview>('GET', `/projects/${id}`),
      call<DaemonPage<DaemonAgent>>('GET', '/agents'),
    ]);
    const detail = toProjectDetail(
      overview,
      new Map(agents.items.map((agent) => [agent.id, agentDisplayName(agent.name)])),
    );
    // A watched project whose provider says which repository it deploys is linked to
    // it, once: the code is the provider's own, so nobody has to choose it again.
    if (
      detail.project.source.kind === 'unlinked' &&
      overview.project.mode === 'watching' &&
      !triedLinking.has(overview.project.id)
    ) {
      triedLinking.add(overview.project.id);
      const deployed = (await reads.forProject(overview.project.id))
        .flatMap(({ details }) => details.deployments)
        .find((deployment) => deployment.repo && deployment.commit);
      if (deployed?.repo && deployed.commit) {
        try {
          await call('PUT', `/projects/${id}/source`, {
            source: { kind: 'github', repository: deployed.repo, commit: deployed.commit },
          });
          detail.project.source = {
            kind: 'github',
            ref: `${deployed.repo}@${deployed.commit.slice(0, 7)}`,
          };
        } catch {
          // Linking is a convenience; the app shows as not linked instead.
        }
      }
    }
    // Deploys that happen on a provider (a watched project, or a piece an agent
    // added at a provider) count too: show whichever is newest.
    detail.project.last_deploy = laterDeploy(
      detail.project.last_deploy,
      await lastProviderDeploy(reads, overview.project.id).catch(() => null),
    );
    // The provider counts a watched project's requests; Hoista's own count is empty there.
    if (detail.stats.requests === null && overview.project.mode === 'watching') {
      const week = await providerWeek(call, overview.resources);
      if (week) {
        detail.stats.requests = week.reduce((total, day) => total + day, 0);
        detail.stats.requests_daily = week.length > 1 ? week : detail.stats.requests_daily;
      }
    }
    // Each resource's latest measured figure, for its row on the overview.
    const details = await Promise.allSettled(
      overview.resources.map((resource) =>
        call<{ latest_usage: Usage | null }>(
          'GET',
          `/resources/${encodeURIComponent(resource.id)}/details`,
        ),
      ),
    );
    detail.resources = detail.resources.map((resource, index) => {
      const read = details[index];
      return read?.status === 'fulfilled'
        ? { ...resource, usage: usageLine(resource, read.value.latest_usage) }
        : resource;
    });

    return detail;
  });
}

/** One short measured figure for a resource row: size for data, requests for the rest. */
function usageLine(resource: Resource, usage: Usage | null): string | null {
  if (!usage) return null;
  if (resource.kind === 'database' || resource.kind === 'bucket') {
    if (usage.storage_bytes === null) return null;
    const bytes = usage.storage_bytes;
    return bytes >= 1e9 ? `${(bytes / 1e9).toFixed(1)} GB` : `${Math.round(bytes / 1e6)} MB`;
  }
  if (usage.requests === null) return null;
  return `${usage.requests.toLocaleString('en-US')} req today`;
}

export function toProjectDetail(
  overview: ProjectOverview,
  agentNames: ReadonlyMap<string, string>,
): ProjectDetail {
  const summary = toProjectListItem(overview, agentNames);
  const days = overview.stats.traffic.map((day) => day.requests);
  return {
    project: { ...summary, source: sourceOf(overview.project.source, overview.source_git) },
    resources: overview.resources.map(toResource),
    stats: {
      requests: overview.stats.requests,
      error_rate: overview.stats.error_rate,
      p99_ms: overview.stats.p99,
      requests_daily: days.every((count): count is number => count !== null) ? days : null,
      requests_change: null,
      // The newest day's count; the card says "today", since days are UTC days.
      errors_24h: overview.stats.traffic.at(-1)?.errors ?? null,
      top_error: null,
      agent_question: questionFor(overview),
    },
  };
}

/**
 * A question worth asking an agent, from what Hoista actually observed:
 * something failing, or server errors today. Null when nothing calls for one.
 */
function questionFor(overview: ProjectOverview): string | null {
  const name = overview.project.name;
  const failing = overview.resources.find((resource) => resource.status === 'failing');
  if (failing) return `Why is ${failing.name} failing in ${name}?`;
  const errors = overview.stats.traffic.at(-1)?.errors ?? 0;
  if (errors > 0) return `Why is ${name} returning server errors today?`;
  return null;
}

function sourceOf(
  source: Source,
  git: ProjectOverview['source_git'],
): ProjectDetail['project']['source'] {
  switch (source.kind) {
    case 'folder':
      return { kind: 'folder', ref: source.path, ...(git ? { git } : {}) };
    case 'github':
      return { kind: 'github', ref: `${source.repository}@${source.commit.slice(0, 7)}` };
    case 'unlinked':
      return { kind: 'unlinked', ref: '' };
  }
}

export function toResource(resource: DaemonResource): Resource {
  return {
    id: resource.id,
    project_id: resource.project_id ?? '',
    kind: resource.kind,
    provider: resource.provider,
    product: productOf(resource),
    name: resource.name,
    region: resource.region,
    status: resource.status,
    provider_ref: resource.provider_ref,
    usage: null,
    url: resource.url,
  };
}

/** The provider's own product name, for labels like `Workers` beside the provider's name. */
function productOf(resource: DaemonResource): string {
  return (
    providerEntry(resource.provider)?.words?.product(resource.kind, resource.provider_ref) ?? ''
  );
}

/** Calendar days left this month, counting today. */
