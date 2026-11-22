/**
 * Servers Hoista made: whether agents with Full access work on one without
 * asking (`GET`/`PUT /resources/{id}/server/agents`), and making a new one in
 * a project (`POST /projects/{id}/servers`).
 */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { projectKeys } from './keys';
import { apiPath, request } from './request';

const serverKeys = {
  agents: (resourceId: string) => [...projectKeys.resource(resourceId), 'server-agents'] as const,
};

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
}

export function useAddServer() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ projectId, accountId, name, serverType, location }: NewServer) =>
      request<{ summary: string }>('POST', apiPath('projects', projectId, 'servers'), {
        account_id: accountId,
        name,
        server_type: serverType,
        location,
      }),
    onSuccess: (_, asked) =>
      queryClient.invalidateQueries({ queryKey: projectKeys.detail(asked.projectId) }),
  });
}
