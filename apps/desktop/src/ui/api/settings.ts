/**
 * React Query hooks for project-wide settings, the Hoista URL, diagnostics
 * and moving to a new computer.
 * Accounts and the profile live in `./accounts`.
 */
import type { VaultInfo } from '@engine-api';
import { useMutation, useQueries, useQuery, useQueryClient } from '@tanstack/react-query';
import type {
  BackupFile,
  DiagnosticReport,
  ExportTransfer,
  ImportTransfer,
  ServerProviders,
  ServerMove,
  ServerRequest,
  ServerSetup,
  RemoteContents,
  RemoteItem,
  RemotesView,
  Settings,
  SettingsPatch,
  TransferFile,
  TransferImported,
} from '@shared/api';
import { agentKeys, settingsKeys } from './keys';
import { showToast } from '@ui/lib/toast';
import { edit, mergeSections, optimistic } from '@ui/lib/optimistic';
import { coverWindow, reloadInto } from '@ui/lib/switching';
import { request } from './request';
import { MINUTE_MS } from '@shared/time';
import { POLL_MS } from './polling';

/** Where tokens are kept, as the vault in use says (`GET /vault`). */
export function useVaultInfo() {
  return useQuery({
    queryKey: [...settingsKeys.all, 'vault'],
    queryFn: () => request<VaultInfo>('GET', '/vault'),
    staleTime: Infinity,
  });
}

export function useSettings() {
  return useQuery({
    queryKey: settingsKeys.all,
    queryFn: () => request<Settings>('GET', '/settings'),
  });
}

/** Saves a change straight away; settings have no Save button. */
export function useUpdateSettings() {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Saved' },
    mutationFn: (patch: SettingsPatch) => request<Settings>('PATCH', '/settings', patch),
    // Switches and choices change on screen at once; the engine's answer then replaces them.
    ...optimistic<SettingsPatch>(queryClient, 'The change', [
      edit<Settings, SettingsPatch>(settingsKeys.all, (old, patch) => mergeSections(old, patch), {
        exact: true,
      }),
    ]),
    onSuccess: (settings) => {
      queryClient.setQueryData(settingsKeys.all, settings);
    },
  });
}

/** Puts Hoista on a Railway account (Touch ID asks first). */
export function useSetUpServer() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: ServerRequest) => request<ServerSetup>('POST', '/server', input),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: settingsKeys.all }),
  });
}

/**
 * Uses a Hoista already running on a server of yours as the remote: its
 * address, checked that it answers.
 */
export function useConnectServer() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (url: string) => request<null>('POST', '/server/connect', { url }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: settingsKeys.all }),
  });
}

/** The providers Hoista can run on, as the engine's adapters say. */
export function useServerProviders() {
  return useQuery({
    queryKey: [...settingsKeys.all, 'server-providers'],
    queryFn: () => request<ServerProviders>('GET', '/server/providers'),
    staleTime: 5 * MINUTE_MS,
  });
}

/** Whether the new server answers yet; asks again while `enabled`. */
export function useServerReady(enabled: boolean) {
  return useQuery({
    queryKey: [...settingsKeys.all, 'server-ready'],
    queryFn: () => request<{ ready: boolean }>('GET', '/server/ready'),
    enabled,
    refetchInterval: (query) => (query.state.data?.ready ? false : POLL_MS.slow),
  });
}

/** Stops showing the server here. It keeps running at the provider. */
export function useForgetServer() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => request<null>('DELETE', '/server'),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: settingsKeys.all }),
  });
}

const remotesKey = ['remotes'] as const;
/** Every query but the remotes list: what changes when the app shows another Hoista. */
const notRemotes = (query: { queryKey: readonly unknown[] }): boolean =>
  query.queryKey[0] !== remotesKey[0];

/**
 * Whether the engine the app shows answers: this Mac's, or the remote being
 * shown. Shared by the banner, the remote strip and the profile dot.
 */
export function useEngineHealth() {
  return useQuery({
    queryKey: ['engine-health'],
    queryFn: () => request<unknown>('GET', '/health'),
    refetchInterval: (query) => (query.state.status === 'error' ? POLL_MS.retry : POLL_MS.health),
    retry: false,
  });
}

/**
 * The remote being shown, and whether it's down: its last check failed, or
 * it stopped answering since. Null while this Mac is shown.
 */
export function useShownRemote(): { remote: RemoteItem; down: boolean } | null {
  const data = useRemotes().data;
  const health = useEngineHealth();
  const remote = data?.items.find((item) => item.id === data.showing);
  if (!remote) return null;
  return { remote, down: remote.reachable === false || health.isError };
}

/**
 * Your remotes and which Hoista the app shows. While this computer waits to
 * be allowed on a remote's page, it checks every 2 seconds; while the remote
 * on screen isn't answering, every 10, so the app notices as soon as it's
 * back; otherwise every 30, so whether each one answers stays fresh.
 */
export function useRemotes() {
  return useQuery({
    queryKey: remotesKey,
    queryFn: () => request<RemotesView>('GET', '/remotes'),
    refetchInterval: (query) => {
      const view = query.state.data;
      if (view?.waiting) return POLL_MS.watching;
      const shown = view?.items.find((item) => item.id === view.showing);
      return shown?.reachable === false ? POLL_MS.slow : POLL_MS.idle;
    },
  });
}

/** Adds a remote by its address; this computer then signs in on its page. */
export function useAddRemote() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { url: string; name?: string }) =>
      request<RemotesView>('POST', '/remotes', input),
    onSuccess: async (view) => {
      queryClient.setQueryData(remotesKey, view);
      await queryClient.invalidateQueries({ queryKey: settingsKeys.all });
    },
  });
}

/** Opens a remote's page to allow this computer; the app finishes by itself. */
export function useSignInRemote() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      request<RemotesView>('POST', `/remotes/${encodeURIComponent(id)}/sign-in`),
    onSuccess: (view) => {
      queryClient.setQueryData(remotesKey, view);
    },
  });
}

/** Cancels a sign-in under way, or forgets this computer's key for a remote. */
export function useSignOutRemote() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      request<RemotesView>('DELETE', `/remotes/${encodeURIComponent(id)}/sign-in`),
    onSuccess: async (view) => {
      queryClient.setQueryData(remotesKey, view);
      await queryClient.invalidateQueries({ predicate: notRemotes });
    },
  });
}

/**
 * Switches instance: this computer's Hoista or a remote's. The window is
 * covered while it happens, then the whole app reloads on its home screen
 * (see lib/switching), so nothing from the instance before is left.
 */
/** How long a switch may take before it gives up. */
const SWITCH_MS = 15_000;

/** Raised when you chose to stay where you were. Nothing to report. */
class StayedPut extends Error {}

/**
 * Switches which Hoista the screens show, then reloads into it. The remote
 * is checked first, so one that isn't running is refused at once with a note
 * saying so, and a switch never waits forever.
 */
export function useShowRemote() {
  const queryClient = useQueryClient();
  const mutation = useMutation({
    mutationFn: async (showing: string) => {
      const list = queryClient.getQueryData<RemotesView>(remotesKey);
      const before = list?.showing ?? 'this_computer';
      const name =
        showing === 'this_computer'
          ? 'This Mac'
          : (list?.items.find((item) => item.id === showing)?.name ?? 'your remote');
      let stay: (() => void) | undefined;
      const stayed = new Promise<never>((_, reject) => {
        stay = () => {
          reject(new StayedPut());
        };
      });
      let timer: ReturnType<typeof setTimeout> | undefined;
      const tooLong = new Promise<never>((_, reject) => {
        timer = setTimeout(() => {
          reject(new Error(`${name} didn’t answer in time. Check it’s running, then try again.`));
        }, SWITCH_MS);
      });
      const uncover = coverWindow(name, () => stay?.());
      const switched = request<RemotesView>('PUT', '/remotes/showing', { showing });
      try {
        const view = await Promise.race([switched, stayed, tooLong]);
        return { view, name };
      } catch (error) {
        uncover();
        // If it got there after all, go back, so the screens match what's shown.
        switched
          .then(() => request<RemotesView>('PUT', '/remotes/showing', { showing: before }))
          .catch(() => undefined);
        throw error;
      } finally {
        clearTimeout(timer);
      }
    },
    onSuccess: ({ name }) => {
      reloadInto(name);
    },
    onError: (error, showing) => {
      void queryClient.invalidateQueries({ queryKey: remotesKey });
      if (error instanceof StayedPut) return;
      showToast(error.message, {
        tone: 'error',
        action: {
          label: 'Try again',
          onClick: () => {
            mutation.mutate(showing);
          },
        },
      });
    },
  });
  return mutation;
}

export function useRenameRemote() {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Renamed' },
    mutationFn: ({ id, name }: { id: string; name: string }) =>
      request<RemotesView>('PATCH', `/remotes/${encodeURIComponent(id)}`, { name }),
    ...optimistic<{ id: string; name: string }>(queryClient, 'The name', [
      edit<RemotesView, { id: string; name: string }>(
        remotesKey,
        (old, { id, name }) => ({
          ...old,
          items: old.items.map((item) => (item.id === id ? { ...item, name } : item)),
        }),
        { exact: true },
      ),
    ]),
    onSuccess: (view) => {
      queryClient.setQueryData(remotesKey, view);
    },
  });
}

/** Forgets a remote on this computer. It keeps running where it is. */
export function useForgetRemote() {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Removed from this computer' },
    mutationFn: (id: string) =>
      request<RemotesView>('DELETE', `/remotes/${encodeURIComponent(id)}`),
    onSuccess: async (view) => {
      queryClient.setQueryData(remotesKey, view);
      await queryClient.invalidateQueries({ predicate: notRemotes });
    },
  });
}

/** Saved backups, newest first. */
export function useBackups() {
  return useQuery({
    queryKey: [...settingsKeys.all, 'backups'],
    queryFn: () => request<BackupFile[]>('GET', '/backups'),
  });
}

/** Saves a backup now. */
export function useBackUpNow() {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Backed up' },
    mutationFn: () => request<BackupFile>('POST', '/backups'),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: [...settingsKeys.all, 'backups'] }),
  });
}

/** Moves everything on this computer to the server; agents here are then paused. */
/** Pushes this computer's projects, accounts and sign-ins to a remote. */
/** What a signed-in remote and this computer hold, read fresh each time it's asked. */
export function useRemoteContents(id: string | null) {
  return useQuery({ ...remoteContents(id ?? ''), enabled: id !== null });
}

/** The same read for several remotes at once, such as the rows of the remotes list. */
export function useRemotesContents(ids: string[]) {
  return useQueries({ queries: ids.map((id) => remoteContents(id)) });
}

function remoteContents(id: string) {
  return {
    queryKey: [...remotesKey, id, 'contents'],
    queryFn: () => request<RemoteContents>('GET', `/remotes/${encodeURIComponent(id)}/contents`),
    staleTime: 0,
    refetchOnMount: 'always' as const,
  };
}

export function usePushToRemote() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      request<ServerMove>('POST', `/remotes/${encodeURIComponent(id)}/push`),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: settingsKeys.all });
      await queryClient.invalidateQueries({ queryKey: remotesKey });
    },
  });
}

/** Installs the waiting update. In the real project this restarts Hoista. */
export function useInstallUpdate() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => request<Settings>('POST', '/updates/install'),
    onSuccess: (settings) => {
      queryClient.setQueryData(settingsKeys.all, settings);
    },
  });
}

export function useCheckForUpdates() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => request<Settings>('POST', '/updates/check'),
    onSuccess: (settings) => {
      queryClient.setQueryData(settingsKeys.all, settings);
    },
  });
}

/**
 * For when something looks wrong: every agent's access is revoked, agent
 * access is paused and waiting requests are dropped. Projects keep running.
 */
export function useStopEverything() {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Every agent is cut off' },
    mutationFn: () => request<{ revoked: number }>('POST', '/agents/stop-all'),
    onSuccess: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: settingsKeys.all }),
        queryClient.invalidateQueries({ queryKey: agentKeys.all }),
      ]),
  });
}

/** Saves a diagnostic report to this computer and returns where it went. */
export function useSaveDiagnosticReport() {
  return useMutation({
    mutationFn: () => request<DiagnosticReport>('POST', '/diagnostics'),
  });
}

/** Saves everything to one passphrase-protected file, for moving to a new computer. */
export function useExportTransfer() {
  return useMutation({
    mutationFn: (body: ExportTransfer) => request<TransferFile>('POST', '/transfer/export', body),
  });
}

/** Restores a transfer file. Everything on screen changes, so every query refetches. */
export function useImportTransfer() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: ImportTransfer) =>
      request<TransferImported>('POST', '/transfer/import', body),
    onSuccess: () => queryClient.invalidateQueries(),
  });
}
