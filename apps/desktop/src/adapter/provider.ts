/**
 * The tabs of a project Hoista watches, filled from what its provider reports live
 * (`GET /resources/{id}/provider`): the provider's own deployment history on Deploys,
 * variable names on Environment, domains on Domains, plus the map and the
 * project's activity. Nothing here is invented or stored. A provider that reports
 * none of this (anything but Railway today) gives empty lists, and the tabs
 * say so; managed projects keep Hoista's own deploys, secrets and domains.
 *
 * Provider reads are cached for a few seconds, so switching tabs doesn't read
 * the provider again for each one.
 */
import { agentDisplayName } from '@shared/agentLogos';
import type {
  ActivityPage,
  Agent as DaemonAgent,
  Deploy as DaemonDeploy,
  ProjectOverview,
  EventDto as DaemonEvent,
  ProviderDetails,
  Link as DaemonLink,
  Resource as DaemonResource,
} from '@engine-api';
import type {
  ActivityItem,
  ActivityRecord,
  ProjectListItem,
  ProjectMap,
  Deploy,
  Domain,
  Page,
  Secret,
} from '@shared/api';
import { providerEntry, SERVICES } from '@shared/providers';
import { RouteError, type Router } from '@shared/router';
import type { Call } from './call';
import { calledTool, isReadOnly, toAgentCall, toolSentence, type CallDetail } from './agents';
import { toResource } from './overview';
import { MINUTE_MS } from '@shared/time';
import { isProviderDeploy, providerDeployId } from '@shared/deployIds';

const CACHE_MS = 10_000;

/** Resources that run code, and so are deployed. */
const CODE_KINDS = new Set<string>(['web', 'service', 'job']);

export interface ProviderReads {
  /** Per watched resource that its provider can describe, its live details. */
  forProject(projectId: string): Promise<{ resource: DaemonResource; details: ProviderDetails }[]>;
  /**
   * Every piece whose provider deploys it: watched ones, and in a project Hoista
   * deploys, the pieces an agent added at a provider that Hoista's own deploys
   * never released (a Vercel site building from its repo).
   */
  providerDeployed(
    projectId: string,
  ): Promise<{ resource: DaemonResource; details: ProviderDetails }[]>;
  forResource(resourceId: string): Promise<ProviderDetails>;
  /** Drops a resource's cached read after Hoista changed something there. */
  forget(resourceId: string): void;
}

export function providerReads(call: Call): ProviderReads {
  const cache = new Map<string, { at: number; value: Promise<ProviderDetails> }>();
  function forResource(resourceId: string): Promise<ProviderDetails> {
    const hit = cache.get(resourceId);
    if (hit && Date.now() - hit.at < CACHE_MS) return hit.value;
    const value = call<ProviderDetails>(
      'GET',
      `/resources/${encodeURIComponent(resourceId)}/provider`,
    );
    cache.set(resourceId, { at: Date.now(), value });
    value.catch(() => cache.delete(resourceId));
    return value;
  }
  async function readAll(
    resources: DaemonResource[],
  ): Promise<{ resource: DaemonResource; details: ProviderDetails }[]> {
    const read = await Promise.allSettled(
      resources.map(async (resource) => ({ resource, details: await forResource(resource.id) })),
    );
    return read.flatMap((result) => (result.status === 'fulfilled' ? [result.value] : []));
  }
  return {
    forResource,
    forget(resourceId) {
      cache.delete(resourceId);
    },
    async forProject(projectId) {
      const overview = await call<ProjectOverview>(
        'GET',
        `/projects/${encodeURIComponent(projectId)}`,
      );
      const watched = overview.resources.filter((resource) => resource.mode === 'watching');
      return readAll(watched);
    },
    async providerDeployed(projectId) {
      const id = encodeURIComponent(projectId);
      const [overview, history] = await Promise.all([
        call<ProjectOverview>('GET', `/projects/${id}`),
        call<{ items: DaemonDeploy[] }>('GET', `/projects/${id}/deploys`),
      ]);
      const released = new Set(
        history.items.flatMap((deploy) => deploy.releases.map((release) => release.resource_id)),
      );
      return readAll(
        overview.resources.filter(
          (resource) =>
            resource.mode === 'watching' ||
            (CODE_KINDS.has(resource.kind) && !released.has(resource.id)),
        ),
      );
    },
  };
}

/**
 * The newest finished deployment the providers report for a project's pieces they
 * deploy themselves, in the shape of a project's "last deploy", or null when
 * there is none.
 */
export async function lastProviderDeploy(
  reads: ProviderReads,
  projectId: string,
): Promise<ProjectListItem['last_deploy']> {
  let newest: ProjectListItem['last_deploy'] = null;
  for (const { resource, details } of await reads.providerDeployed(projectId)) {
    const item = details.deployments.find(
      (deployment) => deployment.state === 'succeeded' || deployment.state === 'failed',
    );
    if (item && (!newest || item.created_at > newest.at)) {
      newest = {
        at: item.created_at,
        status: item.state === 'failed' ? 'failed' : 'live',
        by: item.author
          ? { kind: 'person', name: item.author }
          : { kind: 'system', name: SERVICES[resource.provider].name },
      };
    }
  }
  return newest;
}

/** The later of Hoista's own last deploy and the providers'. */
export function laterDeploy(
  ours: ProjectListItem['last_deploy'],
  providers: ProjectListItem['last_deploy'],
): ProjectListItem['last_deploy'] {
  if (!ours) return providers;
  if (!providers) return ours;
  return providers.at > ours.at ? providers : ours;
}

/** Why a provider stopped a deployment, from the status it reports. */
function stoppedWhy(provider: string, status: string): string {
  switch (status.toUpperCase()) {
    case 'REMOVED':
    case 'REMOVING':
      return `Replaced by a newer deploy on ${provider}.`;
    case 'SKIPPED':
      return `${provider} skipped it: a newer commit was already waiting, or nothing in this service changed.`;
    default:
      return `${provider} stopped it (${status.toLowerCase()}) before it was released.`;
  }
}

/** A provider's deployments as desktop deploys, newest first. The newest success is current. */
export function providerDeploys(
  projectId: string,
  resource: DaemonResource,
  details: ProviderDetails,
): Deploy[] {
  const provider = SERVICES[resource.provider].name;
  const currentId = details.deployments.find((item) => item.state === 'succeeded')?.id;
  return details.deployments.map((item) => ({
    id: providerDeployId(resource.id, item.id),
    project_id: projectId,
    resource_id: resource.id,
    provider: resource.provider,
    version: item.commit ? item.commit.slice(0, 7) : item.id.slice(0, 7),
    source: {
      kind: 'commit',
      ref: [resource.name, item.branch].filter(Boolean).join(' · '),
      message: item.message ?? `${resource.name} deployed on ${provider}`,
      ...(item.repo ? { repo: item.repo } : {}),
      ...(item.commit ? { commit: item.commit } : {}),
    },
    ...(item.provider_page ? { provider_page: item.provider_page } : {}),
    // The commit's author when the provider records one; otherwise the provider itself.
    started_by: item.author
      ? { kind: 'person', name: item.author }
      : { kind: 'system', name: provider },
    // Earlier versions the provider can go back to show as earlier live
    // versions, whatever the provider calls them now (Railway says REMOVED).
    status:
      item.state === 'succeeded' || (item.state === 'stopped' && item.can_roll_back === true)
        ? 'live'
        : item.state === 'failed'
          ? 'failed'
          : item.state === 'in_progress'
            ? 'running'
            : 'cancelled',
    ...(item.can_roll_back === null ? {} : { can_roll_back: item.can_roll_back }),
    steps: [],
    url: item.url,
    created_at: item.created_at,
    duration_ms:
      item.finished_at && item.state !== 'in_progress'
        ? Math.max(0, Date.parse(item.finished_at) - Date.parse(item.created_at)) || null
        : null,
    current: item.id === currentId,
    provider_status: item.status.toUpperCase(),
    failure:
      item.state === 'failed'
        ? {
            step: null,
            message: `${provider} reported ${item.status.toLowerCase()} for this deployment. Its logs are on the Logs tab.`,
            fix: null,
          }
        : item.state === 'stopped' && item.can_roll_back !== true
          ? { step: null, message: stoppedWhy(provider, item.status), fix: null }
          : null,
  }));
}

export function addProviderRoutes(router: Router, call: Call, reads: ProviderReads): void {
  router.on('GET', '/projects/:id/secrets', async ({ params }): Promise<Page<Secret>> => {
    const id = params.id ?? '';
    const watched = await reads.forProject(id);
    if (watched.length === 0) {
      return call<Page<Secret>>('GET', `/projects/${encodeURIComponent(id)}/secrets`);
    }
    const items = watched.flatMap(({ resource, details }) =>
      (details.variable_names ?? []).map((name): Secret => ({
        name,
        resource_id: resource.id,
        resource_name: resource.name,
        set_at: '',
        fingerprint: '',
        // Values stay at the provider; Hoista knows only whether one points at
        // another service there.
        managed_by: null,
        links_to: details.references.find((item) => item.variable === name)?.service ?? null,
        visible_to_browser: false,
        missing: false,
        read_in: null,
        preview: null,
      })),
    );
    // The project's shared variables, once each.
    const seen = new Set<string>();
    for (const { resource, details } of watched) {
      for (const name of details.shared_variable_names ?? []) {
        if (seen.has(name)) continue;
        seen.add(name);
        items.push({
          name,
          resource_id: resource.id,
          resource_name: 'Every service',
          set_at: '',
          fingerprint: '',
          managed_by: null,
          links_to: null,
          visible_to_browser: false,
          missing: false,
          read_in: null,
          preview: null,
          shared: true,
        });
      }
    }
    return { items, cursor: null };
  });

  // Many at once, on a resource or as the project's shared variables.
  router.on('PUT', '/projects/:id/variables', async ({ params, body }) => {
    const projectId = params.id ?? '';
    const input = (body ?? {}) as {
      resource_id?: unknown;
      variables?: unknown;
      shared?: unknown;
      redeploy?: unknown;
    };
    const overview = await call<ProjectOverview>(
      'GET',
      `/projects/${encodeURIComponent(projectId)}`,
    );
    if (overview.project.mode !== 'watching') {
      throw new RouteError('secret.unsupported', 'Add this project’s variables one at a time.');
    }
    const resource = watchedTarget(overview.resources, input.resource_id);
    await call('PUT', `/resources/${encodeURIComponent(resource.id)}/provider/variables`, {
      variables: input.variables,
      shared: input.shared === true,
      redeploy: input.redeploy !== false,
    });
    reads.forget(resource.id);
    return null;
  });

  // A watched project's variables live on its provider: set and delete them there.
  router.on('PUT', '/projects/:id/secrets/:name', async ({ params, body }) => {
    const projectId = params.id ?? '';
    const name = params.name ?? '';
    const input = (body ?? {}) as { value?: unknown; resource_id?: unknown; shared?: unknown };
    const overview = await call<ProjectOverview>(
      'GET',
      `/projects/${encodeURIComponent(projectId)}`,
    );
    if (overview.project.mode !== 'watching') {
      return call(
        'PUT',
        `/projects/${encodeURIComponent(projectId)}/secrets/${encodeURIComponent(name)}`,
        {
          value: input.value,
        },
      );
    }
    const resource = watchedTarget(overview.resources, input.resource_id);
    if (input.shared === true) {
      await call('PUT', `/resources/${encodeURIComponent(resource.id)}/provider/variables`, {
        variables: { [name]: input.value },
        shared: true,
      });
      reads.forget(resource.id);
      return null;
    }
    await call(
      'PUT',
      `/resources/${encodeURIComponent(resource.id)}/provider/variables/${encodeURIComponent(name)}`,
      {
        value: input.value,
      },
    );
    reads.forget(resource.id);
    return null;
  });

  router.on('DELETE', '/projects/:id/secrets/:name', async ({ params, query: search }) => {
    const projectId = params.id ?? '';
    const overview = await call<ProjectOverview>(
      'GET',
      `/projects/${encodeURIComponent(projectId)}`,
    );
    if (overview.project.mode !== 'watching') {
      throw new RouteError('secret.unsupported', 'Remove this variable at its provider.');
    }
    const resource = watchedTarget(overview.resources, search.get('resource'));
    await call(
      'DELETE',
      search.get('shared') === '1'
        ? `/resources/${encodeURIComponent(resource.id)}/provider/shared-variables/${encodeURIComponent(params.name ?? '')}`
        : `/resources/${encodeURIComponent(resource.id)}/provider/variables/${encodeURIComponent(params.name ?? '')}`,
    );
    reads.forget(resource.id);
    return null;
  });

  router.on('GET', '/projects/:id/domains', async ({ params }): Promise<Page<Domain>> => {
    const id = params.id ?? '';
    const [watched, own, overview] = await Promise.all([
      reads.forProject(id),
      // A watched project may have none of Hoista's own; that's no reason to show nothing.
      call<Page<Domain>>('GET', `/projects/${encodeURIComponent(id)}/domains`).catch(
        (): Page<Domain> => ({ items: [], cursor: null }),
      ),
      call<ProjectOverview>('GET', `/projects/${encodeURIComponent(id)}`),
    ]);
    {
      // The provider's own address for each resource it serves (`app.fly.dev`),
      // beside the domains added in Hoista.
      const taken = new Set(own.items.map((domain) => domain.name));
      const providerOwn = overview.resources.flatMap((resource): Domain[] => {
        const name = resource.url?.replace(/^https?:\/\//, '').replace(/\/.*$/, '');
        if (!name || taken.has(name)) return [];
        taken.add(name);
        return [
          {
            name,
            resource_id: resource.id,
            kind: 'default',
            // The address works when what it serves does.
            status: resource.status === 'failing' ? 'error' : 'active',
            role: null,
            redirects_to: null,
            ssl_renews_in_days: null,
            port: null,
            certificate: null,
            dns: [],
            reaches: resource.name,
            setup: null,
          },
        ];
      });
      if (watched.length === 0)
        return { items: [...providerOwn, ...own.items], cursor: own.cursor };
      // A project with both: what each watched provider reports, then every
      // other service's own address and the domains Hoista keeps, once each.
      // Several resources of one provider project report the same address (each
      // Supabase function, its database): list it once, for the first.
      const seen = new Set<string>();
      const reported = watchedDomains().filter((domain) => {
        if (seen.has(domain.name)) return false;
        seen.add(domain.name);
        return true;
      });
      const named = new Set(reported.map((domain) => domain.name));
      return {
        items: [
          ...reported,
          ...[...providerOwn, ...own.items].filter((domain) => !named.has(domain.name)),
        ],
        cursor: null,
      };
    }
    function watchedDomains(): Domain[] {
      const items = watched.flatMap(({ resource, details }) => {
        const primary = details.domains.find(
          (domain) => domain.custom && domain.state === 'working',
        );
        return details.domains.map((domain): Domain => ({
          name: domain.name,
          resource_id: resource.id,
          kind: domain.custom ? 'custom' : 'default',
          status:
            domain.state === 'failed'
              ? 'error'
              : domain.state === 'working' || !domain.custom
                ? 'active'
                : 'pending_dns',
          role: domain === primary ? 'primary' : null,
          redirects_to: null,
          ssl_renews_in_days: null,
          port: domain.port ?? null,
          certificate: domain.certificate ?? null,
          dns: domain.dns.map((record) => ({
            type: record.kind,
            name: record.name,
            value: record.value,
            current: record.current,
            ok: record.ok,
          })),
          reaches: resource.name,
          setup:
            domain.records.length > 0
              ? {
                  dns_host: null,
                  registrar: null,
                  records: domain.records.map((record) => ({
                    type: record.kind,
                    name: record.name,
                    value: record.value,
                  })),
                  nameservers: [],
                  help_url: null,
                }
              : null,
        }));
      });
      return items;
    }
  });

  // "Check now" on a watched project's domain re-reads its provider.
  router.on('POST', '/projects/:id/domains/:name/check', async ({ params }): Promise<Domain> => {
    const projectId = params.id ?? '';
    const name = params.name ?? '';
    const overview = await call<ProjectOverview>(
      'GET',
      `/projects/${encodeURIComponent(projectId)}`,
    );
    if (overview.project.mode !== 'watching') {
      return call<Domain>(
        'POST',
        `/projects/${encodeURIComponent(projectId)}/domains/${encodeURIComponent(name)}/check`,
      );
    }
    for (const resource of overview.resources) reads.forget(resource.id);
    const page = await router.dispatch(
      { method: 'GET', path: `/projects/${encodeURIComponent(projectId)}/domains` },
      () => undefined,
    );
    const found = page.ok
      ? (page.data as Page<Domain>).items.find((domain) => domain.name === name)
      : undefined;
    if (!found)
      throw new RouteError('domain.not_found', 'The provider no longer lists this domain.');
    return found;
  });

  router.on('GET', '/projects/:id/map', async ({ params }): Promise<ProjectMap> => {
    const projectId = params.id ?? '';
    const id = encodeURIComponent(projectId);
    const [map, watched] = await Promise.all([
      call<{ nodes: DaemonResource[]; edges: DaemonLink[] }>('GET', `/projects/${id}/map`),
      reads.forProject(projectId),
    ]);
    const edges = map.edges
      .filter((edge) => !edge.missing)
      .map((edge) => ({ from: edge.from_resource, to: edge.to_resource, via: edge.via }));
    // A provider variable that refers to another service is a real connection:
    // api's DATABASE_URL = ${{Postgres.DATABASE_URL}} means api uses Postgres.
    const byName = new Map(map.nodes.map((node) => [node.name.toLowerCase(), node.id]));
    for (const { resource, details } of watched) {
      for (const reference of details.references) {
        const to = byName.get(reference.service.toLowerCase());
        if (!to || to === resource.id) continue;
        const known = edges.find((edge) => edge.from === resource.id && edge.to === to);
        if (known) {
          if (!known.via.split(', ').includes(reference.variable)) {
            known.via = `${known.via}, ${reference.variable}`;
          }
        } else {
          edges.push({ from: resource.id, to, via: reference.variable });
        }
      }
    }
    // A variable that holds another provider's address (api's DATABASE_URL on
    // Railway pointing at a Supabase database) is a connection too. Each
    // provider's entry says which addresses are its own.
    for (const { resource, details } of watched) {
      for (const address of details.addresses) {
        const project = map.nodes
          .filter((node) => node.id !== resource.id && node.provider !== resource.provider)
          .filter((node) => {
            const entry = providerEntry(node.provider);
            const owned = entry?.owns?.(address);
            return (
              owned !== undefined && owned !== null && owned === entry?.project?.(node.provider_ref)
            );
          });
        // The project's database when it has one, else whatever is first.
        const to = project.find((node) => node.kind === 'database') ?? project[0];
        if (!to) continue;
        const known = edges.find((edge) => edge.from === resource.id && edge.to === to.id);
        if (known) {
          if (!known.via.split(', ').includes(address.variable)) {
            known.via = `${known.via}, ${address.variable}`;
          }
        } else {
          edges.push({ from: resource.id, to: to.id, via: address.variable });
        }
      }
    }
    // A variable whose name belongs to another provider (RESEND_API_KEY,
    // SUPABASE_URL) shows the service uses it, even with no address in it.
    for (const { resource, details } of watched) {
      for (const variable of details.variable_names ?? []) {
        const providers = new Set(
          map.nodes
            .filter((node) => node.provider !== resource.provider)
            .filter((node) => providerEntry(node.provider)?.usedVia?.test(variable))
            .map((node) => node.provider),
        );
        for (const provider of providers) {
          const there = map.nodes.filter((node) => node.provider === provider);
          const to = there.find((node) => node.kind === 'database') ?? there[0];
          if (!to || to.id === resource.id) continue;
          const known = edges.find((edge) => edge.from === resource.id && edge.to === to.id);
          if (known) {
            if (!known.via.split(', ').includes(variable)) known.via = `${known.via}, ${variable}`;
          } else {
            edges.push({ from: resource.id, to: to.id, via: variable });
          }
        }
      }
    }
    // A domain on a DNS host (a Cloudflare zone) leads to the services whose
    // addresses are on it: example.com → api, which serves api.example.com.
    const hostname = (url: string | null): string | null =>
      url
        ? url
            .replace(/^https?:\/\//, '')
            .replace(/[/:].*$/, '')
            .toLowerCase()
        : null;
    const onZone = (name: string, zone: string): boolean =>
      name === zone || name.endsWith(`.${zone}`);
    for (const zone of map.nodes.filter((node) => node.kind === 'domain')) {
      const zoneName = zone.name.toLowerCase();
      for (const node of map.nodes) {
        if (node.id === zone.id || node.kind === 'domain' || node.kind === 'email') continue;
        const names = [
          hostname(node.url),
          ...(watched.find((item) => item.resource.id === node.id)?.details.domains ?? []).map(
            (domain) => domain.name.toLowerCase(),
          ),
        ].filter((name): name is string => name !== null);
        const served = names.find((name) => onZone(name, zoneName));
        if (!served || edges.some((edge) => edge.from === zone.id && edge.to === node.id)) continue;
        edges.push({ from: zone.id, to: node.id, via: served === zoneName ? 'DNS' : served });
      }
    }
    // What each provider deploys from its own repository (a Vercel site that
    // builds on push): its own source box on the map, beside the project's.
    const deployed = await reads.providerDeployed(projectId).catch(() => []);
    const repos = new Map(
      deployed.flatMap(({ resource, details }) => {
        const repo =
          details.config?.repo ?? details.deployments.find((item) => item.repo)?.repo ?? null;
        return repo ? [[resource.id, repo] as const] : [];
      }),
    );
    return {
      nodes: map.nodes.map((node) => ({ ...toResource(node), repo: repos.get(node.id) ?? null })),
      edges,
      // Hoista measures no per-edge traffic.
      traffic: {},
    };
  });

  // Moved boxes are kept by the engine, so the map looks the same anywhere.

  router.on(
    'GET',
    '/projects/:id/activity',
    async ({ params, query: search }): Promise<Page<ActivityItem>> => {
      const project = encodeURIComponent(params.id ?? '');
      const cursor = search.get('cursor');
      const [first, agents] = await Promise.all([
        call<ActivityPage>(
          'GET',
          `/activity?app_id=${project}${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ''}`,
        ),
        call<{ items: DaemonAgent[] }>('GET', '/agents'),
      ]);
      // Status refreshes are recorded too and are hidden here; on a busy
      // project a page can hold nothing else. Read further back until there
      // is something to show (a few pages at most).
      const page: ActivityPage = { items: [...first.items], next_cursor: first.next_cursor };
      const shown = (items: ActivityPage['items']) =>
        items.filter((event) => !HOUSEKEEPING.has(event.kind) && event.kind !== 'agent.tool')
          .length;
      for (let more = 0; more < 6 && page.next_cursor && shown(page.items) < 12; more += 1) {
        const next = await call<ActivityPage>(
          'GET',
          `/activity?app_id=${project}&cursor=${encodeURIComponent(page.next_cursor)}`,
        );
        page.items.push(...next.items);
        page.next_cursor = next.next_cursor;
      }
      const names = new Map(agents.items.map((agent) => [agent.id, agentDisplayName(agent.name)]));
      const items = page.items.map((event) => {
        const item = toActivity(event, names);
        // An agent's change that has no record of its own (setting variables,
        // a provider operation) shows as the call itself; a read stays hidden.
        if (event.kind === 'agent.tool' && item.housekeeping) {
          const { tool } = calledTool(event.summary);
          const recorded = page.items.some(
            (other) =>
              other.kind !== 'agent.tool' &&
              other.actor === event.actor &&
              Math.abs(Date.parse(other.at) - Date.parse(event.at)) < MINUTE_MS,
          );
          if (!isReadOnly(tool) && !recorded) {
            return {
              ...item,
              housekeeping: false,
              title: toolSentence(tool),
              tone: 'good' as const,
            };
          }
        }
        return item;
      });
      // Deploys a provider runs itself (a push to a repository it builds from);
      // add the provider's own record of them.
      if (!search.get('cursor')) {
        for (const { resource, details } of await reads.providerDeployed(params.id ?? '')) {
          for (const deployment of details.deployments.slice(0, 5)) {
            if (deployment.state === 'in_progress') continue;
            const failed = deployment.state === 'failed';
            const version = (deployment.commit ?? deployment.id).slice(0, 7);
            items.push({
              id: providerDeployId(resource.id, deployment.id),
              at: deployment.created_at,
              actor: { kind: 'system', name: SERVICES[resource.provider].name },
              tone: failed ? 'bad' : 'good',
              title: `${failed ? 'Deploy failed' : 'Deployed'} ${version} · ${deployment.message ?? resource.name}`,
              detail: null,
              link: 'deploys',
            });
          }
        }
        items.sort((a, b) => b.at.localeCompare(a.at));
      }
      return { items: items.slice(0, 50), cursor: page.next_cursor };
    },
  );

  // The record behind one activity line. An agent's change is recorded twice:
  // once as what happened, then as the tool call that did it, which keeps
  // what the agent sent. The call is the first one by the same agent after it.
  router.on('GET', '/projects/:id/activity/:item', async ({ params }): Promise<ActivityRecord> => {
    const item = params.item ?? '';
    if (isProviderDeploy(item)) {
      return {
        kind: 'provider.deployment',
        actor_id: 'provider',
        tool: null,
        sent: null,
        status: null,
        happened: [],
        approval_id: null,
      };
    }
    const id = Number(item);
    if (!Number.isSafeInteger(id) || id <= 0) {
      throw new RouteError('resource.not_found', 'That activity is no longer recorded.');
    }
    const around = await call<ActivityPage>('GET', `/activity?cursor=${String(id + 1)}`);
    const event = around.items.find((entry) => entry.id === item);
    if (!event) throw new RouteError('resource.not_found', 'That activity is no longer recorded.');
    const detailOf = async (eventId: string) =>
      (
        await call<{ detail: CallDetail | null }>(
          'GET',
          `/events/${encodeURIComponent(eventId)}/detail`,
        )
      ).detail;
    let detail = await detailOf(event.id);
    if (!detail && event.actor !== 'person' && event.kind !== 'agent.tool') {
      const later = await call<ActivityPage>(
        'GET',
        `/activity?actor=${encodeURIComponent(event.actor)}&cursor=${String(id + 40)}`,
      );
      const tool = later.items
        .filter((entry) => entry.kind === 'agent.tool' && Number(entry.id) > id)
        .sort((a, b) => Number(a.id) - Number(b.id))[0];
      if (tool) detail = await detailOf(tool.id);
    }
    const recorded = toAgentCall(detail);
    return {
      kind: event.kind,
      actor_id: event.actor,
      tool: detail?.tool ?? null,
      ...recorded,
    };
  });

  // Redeploy, restart or deploy the newest commit at the provider; the next read
  // shows the new deployment.
  router.on('POST', '/resources/:id/provider/actions', async ({ params, body }) => {
    const resourceId = params.id ?? '';
    const result = await call<{ summary: string }>(
      'POST',
      `/resources/${encodeURIComponent(resourceId)}/provider/actions`,
      body,
    );
    reads.forget(resourceId);
    return result;
  });

  // Any provider operation may change what the provider reports, so the cached read goes.
  router.on('POST', '/resources/:id/provider/operations/:name', async ({ params, body }) => {
    const resourceId = params.id ?? '';
    const answer = await call<unknown>(
      'POST',
      `/resources/${encodeURIComponent(resourceId)}/provider/operations/${encodeURIComponent(params.name ?? '')}`,
      body,
    );
    reads.forget(resourceId);
    return answer;
  });

  router.on('GET', '/resources/:id/provider', ({ params }) => {
    if (!params.id) throw new RouteError('resource.not_found', 'No such resource.');
    return reads.forResource(params.id);
  });
}

/** The watched resource a variable goes to: the one asked for, else the first that runs code. */
function watchedTarget(resources: DaemonResource[], asked: unknown): DaemonResource {
  const watched = resources.filter((resource) => resource.mode === 'watching');
  const resource =
    watched.find((item) => item.id === asked) ??
    watched.find((item) => item.kind !== 'database') ??
    watched[0];
  if (!resource)
    throw new RouteError('resource.not_found', 'This project has no resource to set it on.');
  return resource;
}

const TONE: [RegExp, ActivityItem['tone']][] = [
  [/(failed|unavailable|error|denied|crash)/, 'bad'],
  [/(alert|warning)/, 'warn'],
  [/(live|finished|connected|merged|created|observed)/, 'good'],
];

/** Events that only record Hoista refreshing what it knows. */
const HOUSEKEEPING = new Set([
  'resource.updated',
  'account.updated',
  'app.updated',
  'import.discovered',
  'logs.opened',
]);

/**
 * Plain titles for events whose summary is only an error code, such as
 * `usage.unavailable` with `resource.not_found`. Never show the codes.
 */
const CODE_TITLES: Record<string, string> = {
  'usage.unavailable': 'Couldn’t read usage from the provider',
  'account.check_unavailable': 'Couldn’t check the account',
  'map.bindings_unavailable': 'Couldn’t read how the resources connect',
  'domain.check_failed': 'Couldn’t check a domain',
  'stream.error': 'Live logs stopped',
  'import.incomplete': 'Some resources couldn’t be read',
  'deploy.rollback_failed': 'Couldn’t roll back the deploy',
  'deploy.resource_recovery_failed': 'Couldn’t recover a resource after a deploy',
  'events.encode_failed': 'An event couldn’t be saved',
};

/** Why, in words, for the error codes these events carry. */
const CODE_WORDS: Record<string, string> = {
  'resource.not_found': 'The provider says the resource isn’t there any more.',
  'account.unauthorized': 'The provider turned the token down. Reconnect the account.',
  'account.forbidden': 'The token isn’t allowed to read this.',
  'provider.rate_limited': 'The provider asked Hoista to slow down. It tries again later.',
  'provider.unavailable': 'The provider didn’t answer.',
  'provider.error': 'The provider returned an error.',
  'network.offline': 'This computer is offline.',
};

const IS_CODE = /^[a-z_]+(\.[a-z_]+)+$/;

/** A sentence for an event kind nobody wrote a title for: `app.renamed` → "Project renamed". */
function kindWords(kind: string): string {
  const words = kind.replace(/[._]/g, ' ').trim();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

function toActivity(event: DaemonEvent, agents: Map<string, string>): ActivityItem {
  const coded = IS_CODE.test(event.summary);
  const actor: ActivityItem['actor'] =
    event.actor === 'person'
      ? { kind: 'person', name: 'You' }
      : agents.has(event.actor)
        ? { kind: 'agent', name: agents.get(event.actor) ?? event.actor }
        : { kind: 'system', name: 'Hoista' };
  return {
    id: event.id,
    at: event.at,
    actor,
    tone: TONE.find(([pattern]) => pattern.test(event.kind))?.[1] ?? 'neutral',
    title: coded ? (CODE_TITLES[event.kind] ?? kindWords(event.kind)) : event.summary,
    detail: coded ? (CODE_WORDS[event.summary] ?? null) : null,
    // An agent's tool call that worked is recorded again as what it did
    // (a deploy, a change); a read that worked changed nothing.
    housekeeping:
      HOUSEKEEPING.has(event.kind) ||
      (event.kind === 'agent.tool' && event.summary.endsWith(': completed')),
    link: event.kind.startsWith('logs.')
      ? 'logs'
      : event.kind.startsWith('deploy.')
        ? 'deploys'
        : event.kind.startsWith('map.')
          ? 'map'
          : null,
  };
}
