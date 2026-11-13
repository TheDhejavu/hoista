/**
 * React Query hooks for projects that already run on your providers: what connecting
 * an account found, adding resources as a project (watching, read-only), and
 * hiding resources you don't want to see.
 */
import { type QueryClient, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import type {
  ProjectDetail,
  ProjectList,
  EmptyProviderProject,
  FoundOnProviders,
  ProviderProject,
  ImportProjectRequest,
} from '@shared/api';

/** The project the engine returns after a rename or split. */
interface SavedProject {
  id: string;
  name: string;
}
import { edit, optimistic } from '@ui/lib/optimistic';
import { projectKeys } from './keys';
import { apiPath, request } from './request';

/**
 * Reloads the project lists without waiting for them: the change is saved,
 * so whoever asked for it can go on at once.
 */
function refreshInBackground(queryClient: QueryClient): void {
  void queryClient.invalidateQueries({ queryKey: projectKeys.all });
}

export function useFoundOnProviders() {
  return useQuery({
    queryKey: projectKeys.found,
    queryFn: () => request<FoundOnProviders>('GET', '/import/found'),
    // A screen that opens to pick from it reads it again, never an old copy.
    refetchOnMount: 'always',
  });
}

/** Adds resources as one project in watching mode. Nothing changes at the provider. */
export function useImportProject() {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Project added. Hoista is watching it' },
    mutationFn: (input: Omit<ImportProjectRequest, 'mode'>) =>
      request<unknown>('POST', '/import/projects', { ...input, mode: 'watching' }),
    onSuccess: () => {
      refreshInBackground(queryClient);
    },
  });
}

/** Adds found resources to a project that already holds their project. Nothing changes at the provider. */
export function useAddToProject() {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Added to the project' },
    mutationFn: ({ projectId, resourceIds }: { projectId: string; resourceIds: string[] }) =>
      request<unknown>('POST', '/import', { project_id: projectId, resource_ids: resourceIds }),
    onSuccess: () => {
      refreshInBackground(queryClient);
    },
  });
}

/** Moves one watched project's resources into another and removes the empty project. Local only. */
export function useCombineProjects() {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Projects combined' },
    mutationFn: ({ from, into }: { from: string; into: string }) =>
      request<unknown>('POST', apiPath('projects', from, 'merge'), { into }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: projectKeys.all }),
  });
}

/** Renames a project in Hoista only. */
export function useRenameProject(projectId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Name saved' },
    mutationFn: (name: string) =>
      request<SavedProject>('PUT', apiPath('projects', projectId, 'name'), { name }),
    ...optimistic<string>(queryClient, 'The name', [
      edit<ProjectDetail, string>(
        projectKeys.detail(projectId),
        (old, name) => ({ ...old, project: { ...old.project, name } }),
        { exact: true },
      ),
      edit<ProjectList, string>(projectKeys.lists(), (old, name) => ({
        ...old,
        items: old.items.map((item) => (item.id === projectId ? { ...item, name } : item)),
      })),
    ]),
    onSettled: () => queryClient.invalidateQueries({ queryKey: projectKeys.all }),
  });
}

/**
 * Moves resources into another project, from whichever project they are in.
 * Nothing changes at the provider. A project left empty with no deploys is removed.
 */
export function useMoveResources() {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Moved to the project' },
    mutationFn: ({ projectId, resourceIds }: { projectId: string; resourceIds: string[] }) =>
      request<{ removed_projects: string[] }>(
        'POST',
        apiPath('projects', projectId, 'resources', 'move'),
        { resource_ids: resourceIds },
      ),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: projectKeys.all }),
  });
}

/** Moves one resource into a new project of its own. */
export function useSplitResource() {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Moved into its own project' },
    mutationFn: ({ resourceId, name }: { resourceId: string; name: string }) =>
      request<SavedProject>('POST', apiPath('resources', resourceId, 'split'), { name }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: projectKeys.all }),
  });
}

/** Every project on a provider, added or not, for the account window. */
export function useProviderProjects(provider: string) {
  return useQuery({
    queryKey: [...projectKeys.found, 'projects', provider],
    queryFn: () =>
      request<ProviderProject[]>(
        'GET',
        `/provider-projects?provider=${encodeURIComponent(provider)}`,
      ),
  });
}

/** The account's projects at its provider that hold nothing yet. */
export function useEmptyProviderProjects(accountId: string) {
  return useQuery({
    queryKey: [...projectKeys.found, 'empty', accountId],
    queryFn: () =>
      request<EmptyProviderProject[]>('GET', apiPath('accounts', accountId, 'empty-projects')),
  });
}

/** Takes one resource out of its project; it goes back to Found on your providers. */
export function useStopWatchingResource() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (resourceId: string) =>
      request<unknown>('POST', apiPath('resources', resourceId, 'stop-watching')),
    onSuccess: () => {
      refreshInBackground(queryClient);
    },
  });
}
