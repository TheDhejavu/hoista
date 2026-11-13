/**
 * The providers offered on Accounts, in order, with what usually runs there,
 * from their provider entries. A provider without an adapter in the engine isn't
 * listed until it has one.
 */
import type { ProviderId } from '@shared/api';
import { PROVIDER_ENTRIES } from '@shared/providers';
import type { ProviderGroup } from '@shared/providers/types';

export type { ProviderGroup };

export interface Provider {
  id: ProviderId;
  /** What people usually run there, in a few words. */
  for: string;
  group: ProviderGroup;
}

/** Section titles for each group, in the order Accounts shows them. */
export const PROVIDER_GROUPS: { id: ProviderGroup; title: string }[] = [
  { id: 'hosting', title: 'Hosting & servers' },
  { id: 'gpu', title: 'GPUs and AI compute' },
  { id: 'databases', title: 'Databases' },
  { id: 'email', title: 'Email' },
  { id: 'code', title: 'Code' },
];

export const PROVIDERS: Provider[] = PROVIDER_ENTRIES.flatMap((entry) =>
  entry.tile ? [{ id: entry.id, ...entry.tile }] : [],
);
