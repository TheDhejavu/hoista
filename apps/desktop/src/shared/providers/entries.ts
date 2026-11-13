/**
 * Every provider the app knows, one file each. Adding a provider is one file here
 * and one line below; what it can do comes from the engine.
 */
import type { CatalogueEntry, ConnectGuide, ServiceId } from '../api';
import { cloudflare } from './cloudflare';
import { fly } from './fly';
import { github } from './github';
import { neon } from './neon';
import { modal } from './modal';
import { netlify } from './netlify';
import { railway } from './railway';
import { render } from './render';
import { resend } from './resend';
import { runpod } from './runpod';
import { supabase } from './supabase';
import type { ProviderEntry } from './types';
import { vercel } from './vercel';

/** In the order the app offers them. */
export const PROVIDER_ENTRIES: readonly ProviderEntry[] = [
  cloudflare,
  fly,
  railway,
  vercel,
  netlify,
  render,
  neon,
  supabase,
  resend,
  runpod,
  modal,
  github,
];

const BY_ID = new Map<string, ProviderEntry>(PROVIDER_ENTRIES.map((entry) => [entry.id, entry]));

/** One provider's entry, or undefined for an id the app doesn't know. */
export function providerEntry(id: string | undefined): ProviderEntry | undefined {
  return id === undefined ? undefined : BY_ID.get(id);
}

/** True for a service id that arrives as text, for example from the URL. */
export function isServiceId(value: string | undefined): value is ServiceId {
  return value !== undefined && BY_ID.has(value);
}

/** The provider's entry, which every service id has. */
export function entryOf(id: ServiceId): ProviderEntry {
  return BY_ID.get(id) ?? cloudflare;
}

export function catalogueEntry(entry: ProviderEntry): CatalogueEntry {
  return {
    id: entry.id,
    name: entry.name,
    ...entry.catalogue,
    service: entry.id,
    availability: 'available',
    recommended_because: null,
  };
}

/** How to connect a provider, before the engine adds its pages (`GET /providers`). */
export type ConnectSteps = Omit<ConnectGuide, 'signup_url' | 'token_url'>;

export function connectGuide(entry: ProviderEntry): ConnectSteps {
  return { service: entry.id, name: entry.name, ...entry.connect };
}
