/**
 * React Query hooks for the profile, connected accounts and the catalogue.
 *
 * Tokens are passed straight to the mutation and sent once. They are never
 * cached: React Query keeps results, and results never contain a token.
 */
import { useMutation, useQuery, useQueryClient, type QueryClient } from '@tanstack/react-query';
import type {
  Account,
  CatalogueEntry,
  ConnectAccount,
  ConnectGuide,
  ConnectResult,
  Page,
  Profile,
  ProfileUpdate,
  ServiceId,
  SignInAccount,
  AccountChoice,
  AccountBill,
  AccountBilling,
  LookupAccounts,
} from '@shared/api';
import { edit, optimistic } from '@ui/lib/optimistic';
import { accountKeys, projectKeys } from './keys';
import { apiPath, request } from './request';
import { MINUTE_MS } from '@shared/time';

/** Connected accounts; `pollMs` re-reads them while something is waiting on them. */
export function useAccounts(pollMs?: number) {
  return useQuery({
    queryKey: accountKeys.all,
    queryFn: () => request<Page<Account>>('GET', '/accounts'),
    refetchInterval: pollMs ?? false,
  });
}

/** What each provider has actually billed so far this period. Empty when no provider gives a bill. */
export function useBills() {
  return useQuery({
    queryKey: [...accountKeys.all, 'bills'],
    queryFn: () => request<AccountBill[]>('GET', '/bills'),
    staleTime: 5 * MINUTE_MS,
  });
}

/** Every connected account's billing: the bill, the plan usage, or why neither. */
export function useBilling() {
  return useQuery({
    queryKey: [...accountKeys.all, 'billing'],
    queryFn: () => request<AccountBilling[]>('GET', '/billing'),
    staleTime: 5 * MINUTE_MS,
  });
}

/** One account, read again from its provider. */
export function fetchAccount(id: string): Promise<Account> {
  return request<Account>('GET', apiPath('accounts', id));
}

export function useAccount(id: string) {
  return useQuery({
    queryKey: accountKeys.detail(id),
    queryFn: () => fetchAccount(id),
  });
}

export function useCatalogue() {
  return useQuery({
    queryKey: accountKeys.catalogue,
    queryFn: () => request<Page<CatalogueEntry>>('GET', '/catalogue'),
  });
}

export function useConnectGuide(service: ServiceId) {
  return useQuery({
    queryKey: accountKeys.guide(service),
    queryFn: () => request<ConnectGuide>('GET', `/catalogue/${service}/connect`),
  });
}

/** Which accounts a token can reach. Nothing is saved and the token is not kept. */
export function useLookupAccounts() {
  return useMutation({
    mutationFn: (lookup: LookupAccounts) =>
      request<AccountChoice[]>('POST', '/accounts/lookup', lookup),
  });
}

/** Checks a token with the service and, if it works, saves it to the keychain. */
export function useConnectAccount() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (connect: ConnectAccount) => request<ConnectResult>('POST', '/accounts', connect),
    // A new provider brings in the projects found on it.
    onSuccess: () => refreshAccountsAndProjects(queryClient),
  });
}

/**
 * Connects by approving Hoista on the service's own page. Resolves once
 * you have clicked Allow there and Hoista has checked what it can see.
 */
export function useSignInAccount() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (signIn: SignInAccount) =>
      request<ConnectResult>('POST', '/accounts/sign-in', signIn),
    onSuccess: () => refreshAccountsAndProjects(queryClient),
  });
}

/** Swaps the token of an account whose token stopped working. */
export function useReplaceToken(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (token: string) =>
      request<ConnectResult>('PUT', apiPath('accounts', id, 'token'), { token }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: accountKeys.all }),
  });
}

/** Removes the account and its keychain entry. Projects keep running at the provider. */
export function useDisconnectAccount(id: string) {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Account removed' },
    mutationFn: () => request<null>('DELETE', apiPath('accounts', id)),
    onSuccess: () => refreshAccountsAndProjects(queryClient),
  });
}

/** The local profile, or null before first-run setup. */
export function useProfile() {
  return useQuery({
    queryKey: accountKeys.profile,
    queryFn: () => request<Profile | null>('GET', '/profile'),
  });
}

/**
 * Starts Hoista when first-run setup is finished or skipped: saves a profile
 * with the name of the person signed in to this computer, which they can
 * change in Settings.
 */
export function useStartHoista() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async () => {
      // Going through first run again keeps the profile you already have.
      const current = await request<Profile | null>('GET', '/profile');
      if (current) return current;
      const suggestion = await request<ProfileUpdate>('GET', '/profile/suggestion');
      return request<Profile>('PUT', '/profile', suggestion);
    },
    // Set it straight away: the Projects screen checks the profile the moment it opens.
    onSuccess: (profile) => {
      queryClient.setQueryData(accountKeys.profile, profile);
    },
  });
}

export function useSaveProfile() {
  const queryClient = useQueryClient();
  return useMutation({
    meta: { toast: 'Profile saved' },
    mutationFn: (update: ProfileUpdate) => request<Profile>('PUT', '/profile', update),
    ...optimistic<ProfileUpdate>(queryClient, 'Your profile', [
      edit<Profile, ProfileUpdate>(
        accountKeys.profile,
        (old, update) => ({ ...old, ...update, initials: initialsOf(update.name) || old.initials }),
        { exact: true },
      ),
    ]),
  });
}

/** Accounts decide which projects Hoista sees, so changing one refreshes both. */
function refreshAccountsAndProjects(queryClient: QueryClient): Promise<unknown> {
  return Promise.all([
    queryClient.invalidateQueries({ queryKey: accountKeys.all }),
    queryClient.invalidateQueries({ queryKey: projectKeys.all }),
    queryClient.invalidateQueries({ queryKey: projectKeys.attention }),
  ]);
}

/** Initials shown until the engine sends its own: "Sam Rivera" → "SR". */
function initialsOf(name: string): string {
  return name
    .trim()
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((word) => word.charAt(0).toUpperCase())
    .join('');
}
