/**
 * React Query hooks for agents, connection requests and approvals.
 */
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import type {
  AgentAddress,
  AgentLink,
  AccessLevel,
  Agent,
  AgentActivity,
  AgentCall,
  AgentUpdate,
  ConnectionRequest,
  Decision,
  Page,
} from '@shared/api';
import { edit, optimistic } from '@ui/lib/optimistic';
import { agentKeys } from './keys';
import { apiPath, queryString, request } from './request';
import { POLL_MS } from './polling';

/** Connected agents; `watch` looks every couple of seconds, while a dialog waits on one. */
export function useAgents({ watch = false }: { watch?: boolean } = {}) {
  return useQuery({
    queryKey: agentKeys.all,
    queryFn: () => request<Page<Agent>>('GET', '/agents'),
    refetchInterval: watch ? POLL_MS.watching : false,
  });
}

/** Recent agent activity, for every agent or just one. */
export function useAgentActivity(agentId?: string) {
  return useQuery({
    queryKey: agentKeys.activity(agentId),
    // Picking another agent keeps the current list up until the new one arrives.
    placeholderData: keepPreviousData,
    queryFn: () =>
      request<Page<AgentActivity>>('GET', `/agents/activity${queryString({ agent: agentId })}`),
  });
}

/** What one agent call sent and how it ended, for the activity panel. */
export function useAgentCall(eventId: string) {
  return useQuery({
    queryKey: [...agentKeys.all, 'call', eventId],
    queryFn: () => request<AgentCall>('GET', apiPath('agents', 'activity', eventId)),
    staleTime: Infinity,
  });
}

/** Saves an agent's limits: its access, which projects and environments, its webhook. */
export function useUpdateAgent() {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Limits saved' },
    mutationFn: ({ agentId, update }: { agentId: string; update: AgentUpdate }) =>
      request<Agent>('PATCH', apiPath('agents', agentId), update),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: agentKeys.all }),
  });
}

export function useSetAgentAccess() {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Access updated' },
    mutationFn: ({ agentId, access }: { agentId: string; access: AccessLevel }) =>
      request<Agent>('PATCH', apiPath('agents', agentId), { access }),
    ...optimistic<{ agentId: string; access: AccessLevel }>(queryClient, 'Access', [
      edit<Page<Agent>, { agentId: string; access: AccessLevel }>(
        agentKeys.all,
        (old, { agentId, access }) => ({
          ...old,
          items: old.items.map((agent) => (agent.id === agentId ? { ...agent, access } : agent)),
        }),
        { exact: true },
      ),
    ]),
  });
}

export function useDisconnectAgent() {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Agent disconnected' },
    mutationFn: (agentId: string) => request<null>('DELETE', apiPath('agents', agentId)),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: agentKeys.all }),
  });
}

/** Chat apps that came in through the Hoista URL and are waiting to be let in. */
export function useConnectionRequests({ watch = false }: { watch?: boolean } = {}) {
  return useQuery({
    queryKey: agentKeys.requests,
    queryFn: () => request<Page<ConnectionRequest>>('GET', '/agents/requests'),
    // While someone waits for an agent to sign in, look every couple of seconds.
    refetchInterval: watch ? POLL_MS.watching : false,
  });
}

export function useAnswerConnectionRequest() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ requestId, decision, access, projects }: { requestId: string } & Decision) =>
      request<null>('POST', apiPath('agents', 'requests', requestId), {
        decision,
        access,
        projects,
      }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: agentKeys.all }),
  });
}

/** MCP over HTTP, for web agents; its address shows once a request from outside reaches it. */
export function useAgentLink() {
  return useQuery({
    queryKey: [...agentKeys.all, 'link'],
    queryFn: () => request<AgentLink>('GET', '/agents/link'),
    refetchInterval: POLL_MS.live,
  });
}

/** Sets the public address by hand, such as the one a tunnel prints. */
export function useSetAgentLink() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (url: string) => request<AgentLink>('PUT', '/agents/link', { url }),
    onSuccess: (link) => {
      queryClient.setQueryData([...agentKeys.all, 'link'], link);
    },
  });
}

/** Forgets its public address, to use a different tunnel or proxy. */
export function useForgetAgentLink() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => request<AgentLink>('DELETE', '/agents/link'),
    meta: { toast: 'Address forgotten' },
    onSuccess: (link) => {
      queryClient.setQueryData([...agentKeys.all, 'link'], link);
    },
  });
}

/** Where agents connect to Hoista, to copy into an agent. */
export function useAgentAddress() {
  return useQuery({
    queryKey: [...agentKeys.all, 'address'],
    queryFn: () => request<AgentAddress | null>('GET', '/agents/address'),
    // The network address follows the setting and the Wi-Fi while Hoista runs.
    refetchInterval: POLL_MS.live,
  });
}
