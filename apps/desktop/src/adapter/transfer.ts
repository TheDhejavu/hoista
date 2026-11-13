/**
 * Moving to a new computer. The engine reads and writes the encrypted file at
 * a path the person picked in the system dialog; this turns its answers into
 * what the dialogs show: what went into the file, and what came back.
 */
import type { TransferFile, TransferImported } from '@shared/api';
import { RouteError, type Router } from '@shared/router';
import type { Call } from './call';

/** What `POST /transfer/inspect` reports about a file. */
interface Inspection {
  created_at: string;
  has_profile: boolean;
  accounts: number;
  credentials: number;
  projects: number;
  resources: number;
  ignored_resources: number;
  activity: number;
}

function count(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

/** What's in the file, in plain words, leaving out what's empty. */
export function contentsOf(file: Inspection): string[] {
  return [
    file.has_profile ? 'Your profile and settings' : null,
    file.projects > 0
      ? `${count(file.projects, 'project', 'projects')} and ${count(file.resources, 'resource', 'resources')}, with their maps`
      : null,
    file.accounts > 0
      ? `${count(file.accounts, 'provider account', 'provider accounts')} and ${count(file.credentials, 'sign-in', 'sign-ins')}`
      : null,
    file.ignored_resources > 0
      ? `${count(file.ignored_resources, 'resource', 'resources')} you chose not to add`
      : null,
    file.activity > 0 ? count(file.activity, 'activity entry', 'activity entries') : null,
  ].filter((line): line is string => line !== null);
}

function input(body: unknown): { path: string; passphrase: string } {
  const value = (body ?? {}) as { path?: unknown; passphrase?: unknown };
  if (typeof value.path !== 'string' || !value.path) {
    throw new RouteError('transfer.path', 'Choose where the file goes.');
  }
  if (typeof value.passphrase !== 'string' || value.passphrase.length < 12) {
    throw new RouteError('transfer.passphrase', 'Use a passphrase of at least 12 characters.');
  }
  return { path: value.path, passphrase: value.passphrase };
}

export function addTransferRoutes(router: Router, call: Call): void {
  router.on('POST', '/transfer/export', async ({ body }): Promise<TransferFile> => {
    const { path, passphrase } = input(body);
    const saved = await call<{ path: string }>('POST', '/transfer/export', { path, passphrase });
    // Read the file back, so what the dialog lists is what's really in it.
    const file = await call<Inspection>('POST', '/transfer/inspect', {
      path: saved.path,
      passphrase,
    });
    return { path: saved.path, created_at: file.created_at, contents: contentsOf(file) };
  });

  router.on('POST', '/transfer/import', async ({ body }): Promise<TransferImported> => {
    const { path, passphrase } = input(body);
    // Opening it first checks the passphrase and the file before anything changes.
    const file = await call<Inspection>('POST', '/transfer/inspect', { path, passphrase });
    await call('POST', '/transfer/restore', { path, passphrase });
    const profile = await call<{ name?: string } | null>('GET', '/profile').catch(() => null);
    return {
      profile_name: profile?.name ?? '',
      projects: file.projects,
      accounts: file.accounts,
    };
  });
}
