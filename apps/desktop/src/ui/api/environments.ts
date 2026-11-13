/**
 * Hoista environments: separate spaces for projects, such as `default` and
 * `production`. Every project is in one; the window shows one at a time, and
 * agents can be limited to some. Nothing changes at any provider.
 */
import { useSyncExternalStore } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import type { Environment } from '@shared/api';
import { edit, optimistic } from '@ui/lib/optimistic';
import { projectKeys } from './keys';
import { apiPath, request } from './request';

/** The environment every project starts in. It can't be removed. */
export const DEFAULT_ENVIRONMENT = 'env_default';

/** For `useProjects`: every project, whichever environment the window shows. */
export const ALL_ENVIRONMENTS = '*';

const environmentKeys = {
  all: ['environments'] as const,
};

export function useEnvironments() {
  return useQuery({
    queryKey: environmentKeys.all,
    queryFn: () => request<Environment[]>('GET', '/environments'),
  });
}

export function useCreateEnvironment() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => request<Environment>('POST', '/environments', { name }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: environmentKeys.all }),
  });
}

export function useRenameEnvironment() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, name }: { id: string; name: string }) =>
      request<Environment>('PATCH', apiPath('environments', id), { name }),
    ...optimistic<{ id: string; name: string }>(queryClient, 'The name', [
      edit<Environment[], { id: string; name: string }>(
        environmentKeys.all,
        (old, { id, name }) => old.map((item) => (item.id === id ? { ...item, name } : item)),
        { exact: true },
      ),
    ]),
  });
}

export function useRemoveEnvironment() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => request<null>('DELETE', apiPath('environments', id)),
    onSuccess: (_, id) => {
      if (currentEnvironment() === id) setCurrentEnvironment(DEFAULT_ENVIRONMENT);
      return queryClient.invalidateQueries({ queryKey: environmentKeys.all });
    },
  });
}

/** Moves a project into another environment. Nothing changes at the provider. */
export function useMoveProject() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ projectId, environmentId }: { projectId: string; environmentId: string }) =>
      request<unknown>('PUT', apiPath('projects', projectId, 'environment'), {
        environment_id: environmentId,
      }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: projectKeys.all }),
  });
}

// The environment this window shows, remembered on this computer.
const KEY = 'hoista.environment';
const listeners = new Set<() => void>();
let current: string = read();

function read(): string {
  try {
    return window.localStorage.getItem(KEY) ?? DEFAULT_ENVIRONMENT;
  } catch {
    return DEFAULT_ENVIRONMENT;
  }
}

function currentEnvironment(): string {
  return current;
}

function setCurrentEnvironment(id: string): void {
  current = id;
  try {
    window.localStorage.setItem(KEY, id);
  } catch {
    // Not remembered: the window opens on default next time.
  }
  for (const listener of listeners) listener();
}

/**
 * The environment the window shows, and the list it comes from. Falls back to
 * default when the remembered one was removed.
 */
export function useCurrentEnvironment(): {
  id: string;
  environment: Environment | null;
  environments: Environment[];
  set: (id: string) => void;
} {
  const id = useSyncExternalStore((listener) => {
    listeners.add(listener);
    return () => listeners.delete(listener);
  }, currentEnvironment);
  const list = useEnvironments().data ?? [];
  const found = list.find((environment) => environment.id === id) ?? null;
  const shown = found ?? list.find((environment) => environment.id === DEFAULT_ENVIRONMENT) ?? null;
  return {
    id: list.length > 0 && !found ? DEFAULT_ENVIRONMENT : id,
    environment: shown,
    environments: list,
    set: setCurrentEnvironment,
  };
}
