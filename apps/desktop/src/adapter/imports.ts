/**
 * What connecting your providers found (`GET /import/found`), from the engine's
 * unassigned resources and its grouping suggestions. Suggestions are evidence
 * (a shared name or hostname), never applied automatically. Adding a project and
 * hiding a resource go to the engine (`POST /import/projects`,
 * `PUT /resources/{id}/ignored`); neither changes anything at the provider. A provider's
 * display name ("Postgres", "My API") becomes a valid project name first.
 */
import { providerEntry, SERVICES } from '@shared/providers';
import type {
  Account as DaemonAccount,
  ProjectOverview,
  ImportProposal,
  Page as DaemonPage,
  Resource as DaemonResource,
} from '@engine-api';
import type {
  FoundCombine,
  FoundGroup,
  FoundOnProviders,
  FoundResource,
  ProviderProject,
} from '@shared/api';
import type { Router } from '@shared/router';
import type { Call } from './call';

export function addImportRoutes(router: Router, call: Call): void {
  router.on('GET', '/import/found', async (): Promise<FoundOnProviders> => {
    const [candidates, suggestions, projects, accounts] = await Promise.all([
      call<DaemonPage<DaemonResource>>('GET', '/import/candidates?ignored=false'),
      call<DaemonPage<ImportProposal>>('GET', '/import/suggestions'),
      call<DaemonPage<ProjectOverview>>('GET', '/projects/summaries'),
      call<DaemonPage<DaemonAccount>>('GET', '/accounts'),
    ]);
    return toFound(candidates.items, suggestions.items, projects.items, accounts.items);
  });
  // Every project on a provider, with what is already in a project and what isn't.
  router.on('GET', '/provider-projects', async ({ query }): Promise<ProviderProject[]> => {
    const provider = query.get('provider');
    const [candidates, projects, accounts] = await Promise.all([
      call<DaemonPage<DaemonResource>>('GET', '/import/candidates?ignored=false'),
      call<DaemonPage<ProjectOverview>>('GET', '/projects/summaries'),
      call<DaemonPage<DaemonAccount>>('GET', '/accounts'),
    ]);
    return toProjects(candidates.items, projects.items, accounts.items, provider);
  });
  router.on('POST', '/resources/:id/stop-watching', ({ params }) =>
    call('POST', `/resources/${encodeURIComponent(params.id ?? '')}/stop-watching`),
  );
  router.on('POST', '/import/projects', ({ body }) => {
    const input = (body ?? {}) as { name?: unknown };
    return call('POST', '/import/projects', {
      ...input,
      name: projectName(typeof input.name === 'string' ? input.name : ''),
    });
  });
}

/**
 * An engine project name from a provider's display name: lowercase letters, digits
 * and single hyphens, at most 63 characters. "My API (prod)" → "my-api-prod".
 */
export function projectName(display: string): string {
  const slug = display
    .normalize('NFKD')
    // Accents come apart into marks under NFKD; drop the marks.
    .replace(/[\u0300-\u036f]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 63)
    .replace(/-+$/, '');
  return slug || 'project';
}

/** The provider project a resource belongs to, where the provider has projects. */
function projectOf(resource: DaemonResource): string | null {
  return providerEntry(resource.provider)?.project?.(resource.provider_ref) ?? null;
}

export function toFound(
  resources: DaemonResource[],
  proposals: ImportProposal[],
  projects: ProjectOverview[] = [],
  accounts: DaemonAccount[] = [],
): FoundOnProviders {
  const labels = new Map(accounts.map((account) => [account.id, account.label]));
  // Which watched project holds each provider project, preferring one that runs code.
  const holders = new Map<string, ProjectOverview>();
  for (const project of projects) {
    if (project.project.mode !== 'watching') continue;
    for (const resource of project.resources) {
      const providerProject = projectOf(resource);
      if (!providerProject) continue;
      const held = holders.get(providerProject);
      const runsCode = resource.kind !== 'database';
      if (!held || (runsCode && held.resources.every((other) => other.kind === 'database'))) {
        holders.set(providerProject, project);
      }
    }
  }
  const combine: FoundCombine[] = [];
  const offered = new Set<string>();
  for (const project of projects) {
    if (project.project.mode !== 'watching') continue;
    for (const resource of project.resources) {
      const providerProject = projectOf(resource);
      const into = providerProject ? holders.get(providerProject) : undefined;
      if (!into || into.project.id === project.project.id || offered.has(project.project.id))
        continue;
      offered.add(project.project.id);
      combine.push({
        from: { id: project.project.id, name: project.project.name },
        into: { id: into.project.id, name: into.project.name },
        provider: resource.provider,
        reason: `Same ${SERVICES[resource.provider].name} project`,
      });
    }
  }

  const unassigned = new Set(resources.map((resource) => resource.id));
  /** The provider's name for the resources in a suggestion: they're on one provider. */
  const hostOf = (ids: string[]): string => {
    const first = resources.find((resource) => resource.id === ids[0]);
    return first ? SERVICES[first.provider].name : 'provider';
  };
  // Only offer groups whose resources are all still free to add.
  const groups: FoundGroup[] = proposals
    .filter((proposal) => proposal.resource_ids.length > 1)
    .filter((proposal) => proposal.resource_ids.every((id) => unassigned.has(id)))
    .map((proposal) => ({
      id: proposal.id,
      label: proposal.label,
      resource_ids: proposal.resource_ids,
      reason:
        proposal.evidence.kind === 'shared_hostname'
          ? `Same address: ${proposal.evidence.value}`
          : proposal.evidence.kind === 'same_project'
            ? `Same ${hostOf(proposal.resource_ids)} project`
            : `Same name: ${proposal.evidence.value}`,
      ambiguous: proposal.ambiguous,
    }));
  const found: FoundResource[] = resources.map((resource) => {
    const providerProject = projectOf(resource);
    const holder = providerProject ? holders.get(providerProject) : undefined;
    return {
      id: resource.id,
      provider: resource.provider,
      kind: resource.kind,
      name: resource.name,
      url: resource.url,
      join: holder ? { project_id: holder.project.id, project_name: holder.project.name } : null,
      account: { id: resource.account_id, label: labels.get(resource.account_id) ?? '' },
      group: resource.group ?? null,
    };
  });
  return { groups, resources: found, combine };
}

/**
 * Projects as the provider groups them, in each connected account: the
 * resources already in projects and the ones still under Found on your providers.
 */
function toProjects(
  free: DaemonResource[],
  projects: ProjectOverview[],
  accounts: DaemonAccount[],
  provider: string | null = null,
): ProviderProject[] {
  const labels = new Map(accounts.map((account) => [account.id, account.label]));
  const held = projects.flatMap((project) =>
    project.resources.map((resource) => ({
      resource,
      project: { id: project.project.id, name: project.project.name },
    })),
  );
  const all = [...held, ...free.map((resource) => ({ resource, project: null }))].filter(
    ({ resource }) => !provider || resource.provider === provider,
  );
  const providerProjects = new Map<string, ProviderProject>();
  for (const { resource, project } of all) {
    const key = resource.group ? `${resource.account_id}:${resource.group}` : resource.id;
    let providerProject = providerProjects.get(key);
    if (!providerProject) {
      providerProject = {
        key,
        provider: resource.provider,
        account: { id: resource.account_id, label: labels.get(resource.account_id) ?? '' },
        name: resource.group ?? resource.name,
        project: null,
        resources: [],
      };
      providerProjects.set(key, providerProject);
    }
    providerProject.project ??= project;
    providerProject.resources.push({
      id: resource.id,
      name: resource.name,
      kind: resource.kind,
      url: resource.url,
      project_id: project?.id ?? null,
    });
  }
  return [...providerProjects.values()];
}
