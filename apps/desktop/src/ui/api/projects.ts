/**
 * React Query hooks for projects and everything inside one project: resources, the map,
 * activity, logs, environment variables and domains.
 */
import {
  keepPreviousData,
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query';
import type {
  ActivityItem,
  ActivityRecord,
  ProjectDetail,
  ProjectList,
  ProjectListQuery,
  ProjectSummary,
  ProjectMap,
  AttentionItem,
  Domain,
  LogLine,
  LogQuery,
  Page,
  ResourceDetail,
  RemoveProject,
  Secret,
  SetSecret,
  SourcePreview,
} from '@shared/api';
import { ALL_ENVIRONMENTS, useCurrentEnvironment } from './environments';
import { projectKeys, providerKeys } from './keys';
import { apiPath, queryString, request } from './request';
import type { ProviderDetails } from '@shared/api';

/**
 * Projects in the environment the window shows, unless the query names one;
 * `environment: ALL_ENVIRONMENTS` reads every project.
 */
export function useProjects(query: ProjectListQuery = {}) {
  const shown = useCurrentEnvironment().id;
  const environment =
    query.environment === ALL_ENVIRONMENTS ? undefined : (query.environment ?? shown);
  const full = { ...query, environment };
  return useQuery({
    queryKey: projectKeys.list(full),
    queryFn: () => request<ProjectList>('GET', `/projects${queryString({ ...full })}`),
    // Changing a filter keeps the current list on screen until the new one arrives.
    placeholderData: keepPreviousData,
  });
}

export function useProject(id: string, options: { enabled?: boolean } = {}) {
  return useQuery({
    queryKey: projectKeys.detail(id),
    queryFn: () => request<ProjectDetail>('GET', apiPath('projects', id)),
    enabled: options.enabled ?? true,
  });
}

/**
 * The project's name, status, address and providers as soon as they're known:
 * from its details once they arrive, or before then from a Projects list
 * already in memory, so its page can show them at once.
 */
export function useProjectSummary(id: string): ProjectSummary | undefined {
  const queryClient = useQueryClient();
  const detail = useProject(id).data;
  if (detail) return detail.project;
  for (const [, list] of queryClient.getQueriesData<ProjectList>({
    queryKey: projectKeys.lists(),
  })) {
    const found = list?.items.find((item) => item.id === id);
    if (found) return found;
  }
  return undefined;
}

export function useProjectMap(id: string) {
  return useQuery({
    queryKey: projectKeys.map(id),
    queryFn: () => request<ProjectMap>('GET', apiPath('projects', id, 'map')),
  });
}

/**
 * Says which service uses a resource, as a line on the map. Only Hoista
 * records it; nothing changes at the provider.
 */
export function useLinkResources(projectId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Linked on the map' },
    mutationFn: (link: { from: string; to: string }) =>
      request<unknown>('POST', '/links', {
        id: '',
        from_resource: link.from,
        to_resource: link.to,
        via: 'uses it',
        derived: false,
        missing: false,
      }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: projectKeys.map(projectId) }),
  });
}

export function useActivity(id: string) {
  return useQuery({
    queryKey: projectKeys.activity(id),
    queryFn: () => request<Page<ActivityItem>>('GET', apiPath('projects', id, 'activity')),
  });
}

/** The whole activity of a project, a page at a time, for its Activity page. */
export function useProjectActivityPages(id: string) {
  return useInfiniteQuery({
    queryKey: [...projectKeys.activity(id), 'all'],
    initialPageParam: null as string | null,
    queryFn: ({ pageParam }) =>
      request<Page<ActivityItem>>(
        'GET',
        `${apiPath('projects', id, 'activity')}${pageParam ? `?cursor=${encodeURIComponent(pageParam)}` : ''}`,
      ),
    getNextPageParam: (last) => last.cursor ?? null,
  });
}

/** The full record behind one activity line, fetched when it is opened. */
export function useActivityRecord(projectId: string, itemId: string) {
  return useQuery({
    queryKey: [...projectKeys.activity(projectId), 'record', itemId],
    queryFn: () =>
      request<ActivityRecord>('GET', apiPath('projects', projectId, 'activity', itemId)),
    staleTime: Infinity,
  });
}

export function useAttention() {
  return useQuery({
    queryKey: projectKeys.attention,
    queryFn: () => request<Page<AttentionItem>>('GET', '/attention'),
  });
}

/**
 * The person's answer for a change Hoista didn't hear back about: they
 * checked the provider, and it happened or it didn't. Either way an identical
 * change can be made again.
 */
export function useCheckOperation() {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Noted. That change can be made again' },
    mutationFn: ({ id, happened }: { id: string; happened: boolean }) =>
      request<unknown>('POST', `/operations/${encodeURIComponent(id)}/check`, { happened }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: projectKeys.attention }),
  });
}

export function useResource(id: string) {
  return useQuery({
    queryKey: projectKeys.resource(id),
    queryFn: () => request<ResourceDetail>('GET', apiPath('resources', id)),
  });
}

/** `live` re-reads every few seconds, for watching lines arrive. */
/** How often live logs are re-read. */
// Live lines stream from the provider into Hoista; this only re-reads them
// locally. History is still read from the provider at most every 30 seconds.
const LIVE_LOGS_MS = 3_000;

export function useLogs(
  projectId: string,
  query: LogQuery,
  { live = false }: { live?: boolean } = {},
) {
  const params = {
    level: query.level,
    source: query.source,
    q: query.q,
    since_minutes: query.since_minutes?.toString(),
    resource: query.resource,
    live: live ? '1' : undefined,
  };
  return useQuery({
    queryKey: projectKeys.logs(projectId, query),
    // A new filter keeps this project's lines up until the new ones arrive;
    // another project's lines are never shown in its place.
    placeholderData: (previous, previousQuery) =>
      previousQuery?.queryKey[1] === projectId ? previous : undefined,
    queryFn: () =>
      request<Page<LogLine>>(
        'GET',
        `${apiPath('projects', projectId, 'logs')}${queryString(params)}`,
      ),
    refetchInterval: live ? LIVE_LOGS_MS : false,
  });
}

export function useSecrets(projectId: string) {
  return useQuery({
    queryKey: projectKeys.secrets(projectId),
    queryFn: () => request<Page<Secret>>('GET', apiPath('projects', projectId, 'secrets')),
  });
}

/** Sets or replaces one variable. The value goes to the provider and is never read back. */
export function useSetSecret(projectId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: (_data, variables) => `${(variables as { name: string }).name} saved` },
    mutationFn: ({
      name,
      value,
      resourceId,
      shared,
    }: { name: string; resourceId?: string; shared?: boolean } & SetSecret) =>
      request<Secret>('PUT', apiPath('projects', projectId, 'secrets', name), {
        value,
        ...(resourceId && { resource_id: resourceId }),
        ...(shared && { shared: true }),
      }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: projectKeys.secrets(projectId) }),
  });
}

export function useDomains(projectId: string) {
  return useQuery({
    queryKey: projectKeys.domains(projectId),
    queryFn: () => request<Page<Domain>>('GET', apiPath('projects', projectId, 'domains')),
  });
}

/** Adds a domain to a project. It starts waiting for DNS, with the records to add. */
export function useAddDomain(projectId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: (_data, variables) => `${String(variables)} added. Waiting for DNS` },
    mutationFn: (name: string) =>
      request<Domain>('POST', apiPath('projects', projectId, 'domains'), { name }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: projectKeys.domains(projectId) }),
  });
}

/** Asks the engine to look up the domain's DNS again now, instead of waiting. */
export function useCheckDomain(projectId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (name: string) =>
      request<Domain>('POST', apiPath('projects', projectId, 'domains', name, 'check')),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: projectKeys.domains(projectId) }),
  });
}

/** Deletes the app from its providers. `backup` saves a copy of its data to this computer first. */
export function useRemoveProject(projectId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Project removed' },
    mutationFn: (body: RemoveProject) =>
      request<null>('DELETE', apiPath('projects', projectId), body),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: projectKeys.all }),
  });
}

/** Removes a watched project's variable at its provider. */
/** Sets many variables at once on a watched project, or its project's shared ones. */
export function useSetVariables(projectId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    meta: {
      toast: (_data, variables) =>
        `${String(Object.keys((variables as { variables: object }).variables).length)} variables saved`,
    },
    mutationFn: (input: {
      resourceId: string;
      variables: Record<string, string>;
      shared: boolean;
      redeploy: boolean;
    }) =>
      request<null>('PUT', apiPath('projects', projectId, 'variables'), {
        resource_id: input.resourceId,
        variables: input.variables,
        shared: input.shared,
        redeploy: input.redeploy,
      }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: projectKeys.secrets(projectId) }),
  });
}

export function useDeleteSecret(projectId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: (_data, variables) => `${(variables as { name: string }).name} removed` },
    mutationFn: ({
      name,
      resourceId,
      shared,
    }: {
      name: string;
      resourceId: string;
      shared?: boolean;
    }) =>
      request<null>(
        'DELETE',
        `${apiPath('projects', projectId, 'secrets', name)}${queryString({ resource: resourceId, shared: shared ? '1' : undefined })}`,
      ),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: projectKeys.secrets(projectId) }),
  });
}

/** What a resource's provider reports about it, for `useQuery` or `useQueries`. */
export function providerDetailsQuery(resourceId: string) {
  return {
    queryKey: providerKeys.detail(resourceId),
    queryFn: () => request<ProviderDetails>('GET', apiPath('resources', resourceId, 'provider')),
  };
}

/** What a resource's provider reports about it; waits while `resourceId` is null. */
export function useProviderDetails(resourceId: string | null) {
  return useQuery({ ...providerDetailsQuery(resourceId ?? ''), enabled: resourceId !== null });
}

/** Links a project to the folder or pinned GitHub commit its code comes from. */
export function useLinkSource(projectId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    meta: {
      toast: (_data, variables) =>
        (variables as { kind: string }).kind === 'unlinked'
          ? 'Source disconnected'
          : 'Source connected',
    },
    mutationFn: (
      source:
        | { kind: 'folder'; path: string }
        | { kind: 'github'; repository: string; commit: string }
        // A watched project only: Hoista stops pointing at the code.
        | { kind: 'unlinked' },
    ) => request<unknown>('PUT', apiPath('projects', projectId, 'source'), { source }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: projectKeys.all }),
  });
}

/** What's in a folder, before connecting it. Reads only. */
export function usePreviewSource(projectId: string) {
  return useMutation({
    mutationFn: (path: string) =>
      request<SourcePreview>('POST', apiPath('projects', projectId, 'source', 'preview'), { path }),
  });
}

/** Stops watching a project: Hoista forgets it; nothing changes at the provider. */
export function useStopWatching(projectId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Stopped watching. Its resources are back under Found on your providers' },
    mutationFn: () => request<null>('POST', apiPath('projects', projectId, 'stop-watching')),
    onSuccess: () => queryClient.invalidateQueries(),
  });
}
