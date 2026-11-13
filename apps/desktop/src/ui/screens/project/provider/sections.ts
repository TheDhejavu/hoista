/**
 * A watched project's provider tab: the sections its provider entry declares (see
 * `shared/providers`), for providers whose engine adapter has provider details.
 */
import type { ProviderSection, ProviderView } from '@shared/providers/types';
import { providerEntry } from '@shared/providers';

export type View = ProviderView;
export type Section = ProviderSection;

export function sectionsFor(provider: string): Section[] {
  return providerEntry(provider)?.sections ?? [];
}
