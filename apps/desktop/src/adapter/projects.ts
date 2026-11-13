/**
 * The Projects list, built from the daemon's `GET /projects/summaries` overviews and
 * `GET /projects/totals`. Only presentation is derived here (names, the
 * sparkline, who deployed); counts come from the engine, and
 * anything the engine has not observed stays null.
 */
import { providerEntry, SERVICES } from '@shared/providers';
import { agentDisplayName } from '@shared/agentLogos';
import type {
  Agent as DaemonAgent,
  ProjectOverview,
  ProjectTotals,
  Deploy as DaemonDeploy,
  Page as DaemonPage,
} from '@engine-api';
import type {
  Actor,
  ProjectList,
  ProjectListItem,
  ProjectListQuery,
  ProviderId,
} from '@shared/api';
import type { Router } from '@shared/router';
import { lastProviderDeploy, laterDeploy, type ProviderReads } from './provider';
import { allPages, query, type Call } from './call';
import { MINUTE_MS, HOUR_MS, DAY_MS, DAY_SECONDS, floorTo } from '@shared/time';

export function addProjectRoutes(router: Router, call: Call, reads: ProviderReads): void {
  router.on('GET', '/projects', async ({ query: params }): Promise<ProjectList> => {
    const filters: ProjectListQuery = {
      environment: params.get('environment') ?? undefined,
      status: (params.get('status') as ProjectListQuery['status']) ?? undefined,
      provider: (params.get('provider') as ProviderId | null) ?? undefined,
      sort: (params.get('sort') as ProjectListQuery['sort']) ?? undefined,
      q: params.get('q') ?? undefined,
    };
    // Every project, not only the first page, so the list and its order are whole.
    const [summaries, totals, agents] = await Promise.all([
      allPages<ProjectOverview>(
        call,
        `/projects/summaries${query({
          environment: filters.environment,
          q: filters.q,
          provider: filters.provider,
          status: filters.status,
          sort: daemonSort(filters.sort),
        })}`,
      ),
      call<ProjectTotals>('GET', `/projects/totals${query({ environment: filters.environment })}`),
      call<DaemonPage<DaemonAgent>>('GET', '/agents'),
    ]);
    const names = new Map(agents.items.map((agent) => [agent.id, agentDisplayName(agent.name)]));
    const page: DaemonPage<ProjectOverview> = { items: summaries, next_cursor: null };
    const list = toProjectList(page, totals, names);
    // Watched projects deploy on their provider; show the provider's newest deploy.
    await Promise.all(
      list.items.map(async (item, index) => {
        const resources = page.items[index]?.resources;
        if (item.mode === 'watching' && resources) {
          item.ships_code = resources.some((resource) =>
            ['web', 'service', 'job'].includes(resource.kind),
          );
        }
        // Deploys that happen on a provider count too: a watched project's, and a
        // piece an agent added at a provider.
        if (
          resources &&
          (item.mode === 'watching' ||
            resources.some((resource) => ['web', 'service', 'job'].includes(resource.kind)))
        ) {
          item.last_deploy = laterDeploy(
            item.last_deploy,
            await lastProviderDeploy(reads, item.id).catch(() => null),
          );
        }
        // Railway counts requests itself; Hoista's own count is empty there.
        if (item.requests_7d === null && item.mode === 'watching' && resources) {
          const week = await providerWeek(call, resources);
          if (week) {
            item.requests_7d = week.reduce((total, day) => total + day, 0);
            item.sparkline = week.length > 1 ? week : item.sparkline;
          }
        }
      }),
    );
    // The engine sorts by name or age. Requests and last deploy come from
    // the summaries and the providers, so sort here: every project is in the list.
    sortItems(list.items, filters.sort);
    return list;
  });
}

const WEEK_CACHE_MS = 5 * MINUTE_MS;
const weeks = new Map<string, { at: number; days: number[] | null }>();

/**
 * Requests per day over the last 7 days, as a watched service's provider reports
 * them (every status summed), or null when the provider doesn't say. Cached for a
 * few minutes so the list doesn't ask the provider on every visit.
 */
export async function providerWeek(
  call: Call,
  resources: readonly { id: string; provider: string; kind: string; mode?: string }[],
): Promise<number[] | null> {
  const service = resources.find(
    (resource) => providerEntry(resource.provider)?.requests && resource.kind !== 'database',
  );
  const operation = service && providerEntry(service.provider)?.requests?.byStatus;
  if (!service || !operation) return null;
  const cached = weeks.get(service.id);
  if (cached && Date.now() - cached.at < WEEK_CACHE_MS) return cached.days;
  const end = floorTo(Date.now(), HOUR_MS);
  let days: number[] | null;
  try {
    const answer = await call<
      { statusCode: number; samples: { ts: number | string; value: number }[] }[]
    >(
      'POST',
      `/resources/${encodeURIComponent(service.id)}/provider/operations/${encodeURIComponent(operation)}`,
      {
        args: {
          startDate: new Date(end - 7 * DAY_MS).toISOString(),
          endDate: new Date(end).toISOString(),
          stepSeconds: DAY_SECONDS,
        },
      },
    );
    const byDay = new Map<string, number>();
    for (const status of answer) {
      for (const sample of status.samples) {
        const key = String(sample.ts);
        byDay.set(key, (byDay.get(key) ?? 0) + sample.value);
      }
    }
    days = [...byDay.entries()].sort(([a], [b]) => (a < b ? -1 : 1)).map(([, count]) => count);
  } catch {
    days = null;
  }
  weeks.set(service.id, { at: Date.now(), days });
  return days;
}

/**
 * Request and last-deploy orders, which the engine doesn't offer: highest
 * first, unknown values last, then by name. The list holds every project, so
 * the order is exact.
 */
function sortItems(items: ProjectListItem[], sort: ProjectListQuery['sort']): void {
  const value = (item: ProjectListItem): number | null => {
    switch (sort) {
      case 'requests':
        return item.requests_7d;
      case 'last_deploy':
        return item.last_deploy ? Date.parse(item.last_deploy.at) : null;
      default:
        return null;
    }
  };
  if (sort !== 'requests' && sort !== 'last_deploy') return;
  items.sort((a, b) => {
    const [x, y] = [value(a), value(b)];
    if (x === y) return a.name.localeCompare(b.name);
    if (x === null) return 1;
    if (y === null) return -1;
    return y - x;
  });
}

function daemonSort(sort: ProjectListQuery['sort']): 'name' | 'newest' {
  return sort === 'name' ? 'name' : 'newest';
}

function toProjectList(
  page: DaemonPage<ProjectOverview>,
  totals: ProjectTotals,
  agentNames: ReadonlyMap<string, string>,
): ProjectList {
  return {
    items: page.items.map((overview) => toProjectListItem(overview, agentNames)),
    cursor: page.next_cursor,
    totals: {
      projects: totals.projects,
      live: totals.live,
      failing: totals.failing,
      providers: totals.providers,
    },
  };
}

/** Addresses a provider gives every service, as opposed to the person's own domain. */
const GENERATED =
  /\.(up\.railway\.app|railway\.app|workers\.dev|pages\.dev|fly\.dev|vercel\.app|netlify\.app|onrender\.com|modal\.run|runpod\.net|supabase\.co|neon\.tech)$/i;

/**
 * The project's address: what people open, so a website before an API or a
 * GPU endpoint; and the person's own domain before a provider's address. A
 * service added later never takes over from the domain people already use.
 */
export function projectUrl(
  resources: readonly { url: string | null; kind?: string }[],
): string | null {
  const hostOf = (url: string) => url.replace(/^https?:\/\//, '').split('/')[0] ?? '';
  const ranked = [...resources]
    .filter((resource): resource is { url: string; kind?: string } => resource.url !== null)
    .sort((a, b) => rank(a) - rank(b));
  function rank(resource: { url: string; kind?: string }): number {
    const site = resource.kind === undefined || resource.kind === 'web' ? 0 : 2;
    return site + (GENERATED.test(hostOf(resource.url)) ? 1 : 0);
  }
  return ranked[0]?.url ?? null;
}

export function toProjectListItem(
  overview: ProjectOverview,
  agentNames: ReadonlyMap<string, string>,
): ProjectListItem {
  const { project, resources, stats } = overview;
  const providers = [...new Set(resources.map((resource) => resource.provider))];
  const days = stats.traffic.map((day) => day.requests);
  const sparkline = days.every((count): count is number => count !== null) ? days : null;
  return {
    id: project.id,
    name: project.name,
    url: projectUrl(resources),
    status: overview.status,
    idle_days: null,
    mode: project.mode,
    environment_id: project.environment_id,
    imported: project.mode === 'watching',
    runs_on: providers.map((provider) => SERVICES[provider].name),
    providers,
    requests_7d: stats.requests,
    sparkline: sparkline && sparkline.length > 0 ? sparkline : null,
    last_deploy: overview.last_deploy ? lastDeploy(overview.last_deploy, agentNames) : null,
  };
}

/** Only finished attempts are shown on the list; the engine already skips a running deploy for the last result. */
function lastDeploy(
  deploy: DaemonDeploy,
  agentNames: ReadonlyMap<string, string>,
): ProjectListItem['last_deploy'] {
  if (deploy.status === 'queued' || deploy.status === 'running') return null;
  return {
    at: deploy.finished_at ?? deploy.started_at,
    status: deploy.status === 'failed' ? 'failed' : 'live',
    by: actorOf(deploy, agentNames),
  };
}

/** `person` is you; anything else is an agent id, named as it was when it asked. */
export function actorOf(deploy: DaemonDeploy, agentNames: ReadonlyMap<string, string>): Actor {
  if (deploy.started_by === 'person') return { kind: 'person', name: 'You' };
  return {
    kind: 'agent',
    name: deploy.started_by_name ?? agentNames.get(deploy.started_by) ?? 'An agent',
  };
}
