/**
 * Live engine events, and the data each one refreshes.
 */
import { useEffect } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import type { EngineEvent } from '@shared/api';
import { getBridge } from './bridge';
import { accountKeys, agentKeys, projectKeys, deployKeys, settingsKeys } from './keys';

/**
 * Listens to engine events for the life of the app and refreshes the data
 * each event affects. Mount once, near the root.
 */
export function useEngineEvents(): void {
  const queryClient = useQueryClient();

  useEffect(() => {
    function refresh(event: EngineEvent): void {
      for (const queryKey of keysAffectedBy(event)) {
        void queryClient.invalidateQueries({ queryKey });
      }
    }
    return getBridge().onEvent(refresh);
  }, [queryClient]);
}

/**
 * What one event makes stale, chosen by the area its `kind` belongs to
 * (`deploy.step` → deploy). Unknown kinds refresh nothing, so a newer daemon
 * can add kinds without breaking the app.
 */
export function keysAffectedBy(event: EngineEvent): (readonly unknown[])[] {
  const area = event.kind.split('.')[0];
  const project = event.project_id ? [projectKeys.detail(event.project_id)] : [];
  switch (area) {
    case 'deploy':
      // The recent-deploys key is a prefix of every deploy's key, so this refreshes them all.
      return [deployKeys.recent, projectKeys.lists(), ...project, projectKeys.attention];
    case 'project':
    case 'resource':
    case 'domain':
    case 'secret':
    case 'map':
    case 'usage':
    case 'database':
    case 'provider':
      // A change run at a provider (`provider.operation`) can add or change what's there.
      return [projectKeys.all, projectKeys.attention];
    case 'approval':
      return [agentKeys.approvals, agentKeys.all, projectKeys.attention];
    case 'agent':
      return [agentKeys.all];
    case 'account':
    case 'import':
      return [accountKeys.all, projectKeys.all];
    case 'profile':
      return [accountKeys.profile];
    case 'operation':
      return [projectKeys.attention, projectKeys.all];
    case 'backup':
      return [[...settingsKeys.all, 'backups']];
    case 'link':
    case 'transfer':
      return [settingsKeys.all, agentKeys.all];
    default:
      return [];
  }
}
