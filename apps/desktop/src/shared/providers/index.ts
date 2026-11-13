/**
 * Names and logos of the services Hoista connects to, from their provider
 * entries (`entries.ts`).
 *
 * Logo paths are from Simple Icons (CC0). Their brand colours are data, the
 * one place raw colours are allowed outside the design tokens.
 */
import type { ProviderId, ServiceId } from '../api';
import { PROVIDER_ENTRIES, entryOf, isServiceId } from './entries';
import { OTHER_LOGOS } from './others';
import type { Logo } from './types';

export * from './entries';

/** What the app shows for a service an account can be connected for. */
interface ServiceInfo {
  name: string;
  logo: Logo;
  /** Hoista can read existing projects from it and bring them in. */
  canImport: boolean;
}

export const SERVICES = Object.fromEntries(
  PROVIDER_ENTRIES.map((entry) => [
    entry.id,
    { name: entry.name, logo: entry.logo, canImport: entry.canImport },
  ]),
) as Record<ServiceId, ServiceInfo>;

/** The display name of a service, as it writes it. */
export function serviceName(service: ServiceId): string {
  return entryOf(service).name;
}

/** The logo for any catalogue id, connectable or not, or null if there isn't one. */
export function logoFor(id: string): Logo | null {
  if (isServiceId(id)) return entryOf(id).logo;
  return OTHER_LOGOS[id] ?? null;
}

/** Hosting providers, in the order the app lists them. */
export const PROVIDER_IDS: readonly ProviderId[] = PROVIDER_ENTRIES.filter(
  (entry) => entry.catalogue.category !== 'code',
).map((entry) => entry.id);
