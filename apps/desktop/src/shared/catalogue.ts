/**
 * The catalogue of services shown on the Accounts screen, and how to make a
 * token for each one, from their provider entries (`shared/providers`).
 */
import type { CatalogueEntry, ServiceId } from './api';
import { PROVIDER_ENTRIES, catalogueEntry, connectGuide, type ConnectSteps } from './providers';
import { OTHER_SERVICES } from './providers/others';

export type { ConnectSteps };

export const catalogue: CatalogueEntry[] = [
  ...PROVIDER_ENTRIES.map(catalogueEntry),
  ...OTHER_SERVICES,
];

export const connectGuides = Object.fromEntries(
  PROVIDER_ENTRIES.map((entry) => [entry.id, connectGuide(entry)]),
) as Record<ServiceId, ConnectSteps>;

/**
 * How to name the account when a token cannot list its accounts, so the
 * person can enter it directly.
 */
export const accountIdFields: Partial<Record<ServiceId, { label: string; hint: string }>> =
  Object.fromEntries(
    PROVIDER_ENTRIES.flatMap((entry) => (entry.accountId ? [[entry.id, entry.accountId]] : [])),
  );
