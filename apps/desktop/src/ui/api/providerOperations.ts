/**
 * A provider's whole API through Hoista: what it can do, reads as queries, and
 * changes as mutations. The engine fills in project, environment, service
 * and workspace, removes secrets from answers, and records every change.
 */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import type { ProviderOperation, ProviderQuickAction, ProviderTarget } from '@shared/api';
import { projectKeys, deployKeys, providerKeys } from './keys';
import { apiPath, request } from './request';
import { MINUTE_MS } from '@shared/time';

function base(target: ProviderTarget): string[] {
  return [
    target.kind === 'resource' ? 'resources' : 'accounts',
    target.id,
    'provider',
    'operations',
  ];
}

const providerOpKeys = {
  all: ['provider-operations'] as const,
  list: (target: ProviderTarget) => ['provider-operations', target.kind, target.id] as const,
  read: (target: ProviderTarget, name: string, args: unknown) =>
    ['provider-operations', target.kind, target.id, name, JSON.stringify(args)] as const,
};

export function useProviderOperations(target: ProviderTarget | null) {
  return useQuery({
    queryKey: target ? providerOpKeys.list(target) : ['provider-operations', 'none'],
    enabled: target !== null,
    staleTime: 10 * MINUTE_MS,
    queryFn: () => request<ProviderOperation[]>('GET', apiPath(...base(target as ProviderTarget))),
  });
}

/** The buttons a resource's provider offers for it, from the provider's manifest. */
export function useProviderQuickActions(target: ProviderTarget | null) {
  return useQuery({
    queryKey: target
      ? ['provider-quick-actions', target.kind, target.id]
      : ['provider-quick-actions', 'none'],
    enabled: target?.kind === 'resource',
    staleTime: 10 * MINUTE_MS,
    queryFn: () =>
      request<ProviderQuickAction[]>(
        'GET',
        apiPath('resources', (target as ProviderTarget).id, 'provider', 'quick-actions'),
      ),
  });
}

/** Runs a read operation; refreshes once a minute while shown. */
export function useProviderRead<T = unknown>(
  target: ProviderTarget | null,
  name: string,
  args: Record<string, unknown> = {},
  enabled = true,
) {
  return useQuery({
    queryKey: target
      ? providerOpKeys.read(target, name, args)
      : ['provider-operations', 'none', name],
    enabled: target !== null && enabled,
    staleTime: MINUTE_MS,
    retry: false,
    queryFn: () => request<T>('POST', apiPath(...base(target as ProviderTarget), name), { args }),
  });
}

/** Runs a change; afterwards every provider read is refreshed. */
export function useRunProviderOperation(target: ProviderTarget | null) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ name, args }: { name: string; args: Record<string, unknown> }) =>
      request<unknown>('POST', apiPath(...base(target as ProviderTarget), name), { args }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: providerOpKeys.all });
      void queryClient.invalidateQueries({ queryKey: providerKeys.all });
      // A change at the provider can show up anywhere: deploys, domains, variables.
      void queryClient.invalidateQueries({ queryKey: projectKeys.all });
      void queryClient.invalidateQueries({ queryKey: deployKeys.recent });
    },
  });
}
