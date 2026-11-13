/** Repositories on a code account (GitHub), and linking a project to one. */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import type { ProviderRepository } from '@engine-api';
import { projectKeys } from './keys';
import { apiPath, request } from './request';
import { MINUTE_MS } from '@shared/time';

export function useRepositories(accountId: string | null) {
  return useQuery({
    queryKey: ['repositories', accountId],
    enabled: accountId !== null,
    staleTime: MINUTE_MS,
    retry: false,
    queryFn: () =>
      request<ProviderRepository[]>('GET', apiPath('accounts', accountId ?? '', 'repositories')),
  });
}

/** A repository's branches, through the GitHub account that reaches it. */
export function useBranches(accountId: string | null, repository: string | null) {
  return useQuery({
    queryKey: ['branches', accountId, repository],
    enabled: accountId !== null && repository !== null,
    staleTime: MINUTE_MS,
    retry: false,
    queryFn: () =>
      request<string[]>(
        'GET',
        `${apiPath('accounts', accountId ?? '', 'branches')}?repository=${encodeURIComponent(repository ?? '')}`,
      ),
  });
}

/** Links the app to a repository at the newest commit of a branch (its default when unset). */
export function useLinkRepository(projectId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Code linked' },
    mutationFn: ({
      accountId,
      repository,
      branch,
    }: {
      accountId: string;
      repository: string;
      branch?: string;
    }) =>
      request<unknown>('PUT', apiPath('projects', projectId, 'repository'), {
        account_id: accountId,
        repository,
        ...(branch ? { branch } : {}),
      }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: projectKeys.all }),
  });
}

/** A new project at a provider (Railway builds it), from a repository. */
export function useNewProjectAtProvider() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      accountId,
      name,
      repository,
      branch,
    }: {
      accountId: string;
      name: string;
      repository: string;
      branch?: string;
    }) =>
      request<{ project: { id: string; name: string }; summary: string }>(
        'POST',
        apiPath('accounts', accountId, 'projects'),
        { name, repository, ...(branch ? { branch } : {}) },
      ),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: projectKeys.all }),
  });
}
