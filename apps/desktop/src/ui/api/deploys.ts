/**
 * React Query hooks for watching deploys: the latest ones, one project's history,
 * and a single deploy's progress, build output and summary. Starting and
 * importing are done by agents over MCP, so there are no hooks for them.
 * Stopping a running deploy and rolling back to an earlier version are the
 * changes made from here.
 *
 * Progress needs no polling: engine events refresh every key under
 * `['deploys']`, which covers each deploy, its log and its summary.
 */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import type { Deploy, DeployLogLine, DeploySummary, Page } from '@shared/api';
import { projectKeys, deployKeys, providerKeys } from './keys';
import { apiPath, request } from './request';
import { isProviderDeploy } from '@shared/deployIds';
import { POLL_MS } from './polling';

/** The latest deploys across every project, newest first. */
/** While a deploy is under way, re-read often enough to see it finish. */
const RUNNING_MS = 10_000;

function whileRunning(page: Page<Deploy> | undefined): number | false {
  return page?.items.some((deploy) => deploy.status === 'running' || deploy.status === 'queued')
    ? RUNNING_MS
    : false;
}

export function useRecentDeploys() {
  return useQuery({
    queryKey: deployKeys.recent,
    queryFn: () => request<Page<Deploy>>('GET', '/deploys'),
    refetchInterval: (query) => whileRunning(query.state.data),
  });
}

/** One project's deploys, Hoista's and its providers', newest first. */
export function fetchDeploys(projectId: string): Promise<Page<Deploy>> {
  return request<Page<Deploy>>('GET', apiPath('projects', projectId, 'deploys'));
}

export function useDeploys(projectId: string) {
  return useQuery({
    queryKey: projectKeys.deploys(projectId),
    queryFn: () => fetchDeploys(projectId),
    refetchInterval: (query) => whileRunning(query.state.data),
  });
}

/** A provider's deploy sends Hoista no events, so its page asks again while it builds. */
function providerDeployRunning(id: string, deploy: Deploy | undefined): boolean {
  return isProviderDeploy(id) && (deploy?.status === 'queued' || deploy?.status === 'running');
}

export function useDeploy(id: string) {
  return useQuery({
    queryKey: deployKeys.detail(id),
    queryFn: () => request<Deploy>('GET', apiPath('deploys', id)),
    refetchInterval: (query) =>
      providerDeployRunning(id, query.state.data) ? POLL_MS.live : false,
  });
}

/** Stops a deploy before it goes live. It ends `cancelled` after the step it's on. */
export function useCancelDeploy() {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Deploy stopped' },
    mutationFn: (id: string) => request<Deploy>('POST', apiPath('deploys', id, 'cancel')),
    onSuccess: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: deployKeys.recent }),
        queryClient.invalidateQueries({ queryKey: projectKeys.all }),
      ]),
  });
}

/** The deploy's log; a provider's build log is read again every few seconds while it runs. */
export function useDeployLog(id: string, running = false) {
  return useQuery({
    queryKey: deployKeys.log(id),
    queryFn: () => request<{ lines: DeployLogLine[] }>('GET', apiPath('deploys', id, 'log')),
    refetchInterval: running && isProviderDeploy(id) ? POLL_MS.live : false,
  });
}

export function useDeploySummary(id: string) {
  return useQuery({
    queryKey: deployKeys.summary(id),
    queryFn: () => request<DeploySummary>('GET', apiPath('deploys', id, 'summary')),
  });
}

/** Puts an earlier version that went live back in front of traffic. Nothing is rebuilt. */
export function useRollBack() {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Rolling back. Live in about a minute' },
    mutationFn: (id: string) =>
      request<{ deploy_id: string }>('POST', apiPath('deploys', id, 'rollback')),
    onSuccess: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: deployKeys.recent }),
        queryClient.invalidateQueries({ queryKey: projectKeys.all }),
      ]),
  });
}

/** What can be asked of a watched resource's provider. */
type ProviderAction = 'redeploy' | 'restart' | 'deploy_latest' | 'sleep' | 'remove_service';

const PROVIDER_ACTION_TOASTS: Record<ProviderAction, string> = {
  redeploy: 'Redeploying. The new deployment shows here in a moment',
  restart: 'Restarting. Back up in a few seconds',
  deploy_latest: 'Deploying the newest commit',
  sleep: 'Saved',
  remove_service: 'Deleted at the provider',
};

/** Asks a watched resource's provider to redeploy, restart or deploy its newest commit. */
export function useProviderAction() {
  const queryClient = useQueryClient();
  return useMutation({
    meta: {
      // A delete says what went, in the engine's words.
      toast: (data, variables) => {
        const action = (variables as { action: ProviderAction }).action;
        const summary = (data as { summary?: string } | undefined)?.summary;
        return action === 'remove_service' && summary ? summary : PROVIDER_ACTION_TOASTS[action];
      },
    },
    mutationFn: ({
      resourceId,
      action,
      on,
      confirmationName,
    }: {
      resourceId: string;
      action: ProviderAction;
      on?: boolean;
      /** For `remove_service`: the resource's name as the person typed it. */
      confirmationName?: string;
    }) =>
      request<{ summary: string }>(
        'POST',
        apiPath('resources', resourceId, 'provider', 'actions'),
        action === 'sleep'
          ? { action, on: on ?? false }
          : action === 'remove_service'
            ? { action, confirmation_name: confirmationName ?? '' }
            : { action },
      ),
    onSuccess: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: deployKeys.recent }),
        queryClient.invalidateQueries({ queryKey: projectKeys.all }),
        queryClient.invalidateQueries({ queryKey: providerKeys.all }),
      ]),
  });
}

/**
 * Any other change at a watched resource's provider, by its action body, such
 * as `{ action: 'add_domain', domain, port }`. The toast is the provider's own
 * summary of what it did.
 */
export function useProviderChange() {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: (data) => (data as { summary?: string }).summary ?? 'Done' },
    mutationFn: ({ resourceId, body }: { resourceId: string; body: Record<string, unknown> }) =>
      request<{ summary: string }>(
        'POST',
        apiPath('resources', resourceId, 'provider', 'actions'),
        body,
      ),
    onSuccess: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: projectKeys.all }),
        queryClient.invalidateQueries({ queryKey: providerKeys.all }),
      ]),
  });
}
