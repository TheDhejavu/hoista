/**
 * Servers Hoista made: whether agents with Full access work on one without
 * asking (`GET`/`PUT /resources/{id}/server/agents`), and making a new one in
 * a project (`POST /projects/{id}/servers`).
 */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import type { Resource } from '@shared/api';
import { projectKeys } from './keys';
import { POLL_MS } from './polling';
import { apiPath, request } from './request';

const serverKeys = {
  agents: (resourceId: string) => [...projectKeys.resource(resourceId), 'server-agents'] as const,
  activity: (resourceId: string) =>
    [...projectKeys.resource(resourceId), 'server-activity'] as const,
};

/** One thing done on a server, as the engine records it. */
export interface ServerEvent {
  id: string;
  at: string;
  /** `person`, `engine`, or an agent's id. */
  actor: string;
  kind: string;
  summary: string;
}

/** What was done on a server, newest first: commands, files, deploy keys, its setup. */
export function useServerActivity(resourceId: string) {
  return useQuery({
    queryKey: serverKeys.activity(resourceId),
    queryFn: () =>
      request<{ items: ServerEvent[] }>(
        'GET',
        apiPath('resources', resourceId, 'server', 'activity'),
      ),
  });
}

/**
 * A server just made, looked at every few seconds until it has set itself
 * up: its resource as the project lists it.
 */
export function useNewServer(projectId: string, resourceId: string | null) {
  return useQuery({
    queryKey: [...projectKeys.detail(projectId), 'new-server', resourceId],
    enabled: resourceId !== null,
    queryFn: async () => {
      const detail = await request<{ resources: Resource[] }>(
        'GET',
        apiPath('projects', projectId),
      );
      return detail.resources.find((resource) => resource.id === resourceId) ?? null;
    },
    refetchInterval: (query) =>
      query.state.data?.status === 'live' || query.state.data?.status === 'failing'
        ? false
        : POLL_MS.watching,
  });
}

interface ServerAgents {
  full_access: boolean;
}

export function useServerAgents(resourceId: string) {
  return useQuery({
    queryKey: serverKeys.agents(resourceId),
    queryFn: () =>
      request<ServerAgents>('GET', apiPath('resources', resourceId, 'server', 'agents')),
  });
}

export function useSetServerAgents(resourceId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Saved' },
    mutationFn: (fullAccess: boolean) =>
      request<ServerAgents>('PUT', apiPath('resources', resourceId, 'server', 'agents'), {
        full_access: fullAccess,
      }),
    onSuccess: (saved) => {
      queryClient.setQueryData(serverKeys.agents(resourceId), saved);
    },
  });
}

export interface NewServer {
  projectId: string;
  accountId: string;
  name: string;
  serverType: string | null;
  location: string | null;
  volumeGb: number | null;
  backups: boolean;
}

export function useAddServer() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      projectId,
      accountId,
      name,
      serverType,
      location,
      volumeGb,
      backups,
    }: NewServer) =>
      request<{ summary: string; resources: { id: string; kind: string }[] }>(
        'POST',
        apiPath('projects', projectId, 'servers'),
        {
          account_id: accountId,
          name,
          server_type: serverType,
          location,
          volume_gb: volumeGb,
          backups,
        },
      ),
    onSuccess: (_, asked) =>
      queryClient.invalidateQueries({ queryKey: projectKeys.detail(asked.projectId) }),
  });
}
