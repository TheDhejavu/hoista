/**
 * What each provider's own pages look like when you make an account and a
 * token there, for the step-by-step provider guides, from their provider entries.
 * The page drawings use these words and addresses; they are pictures of the
 * provider's site, not the site itself.
 */
import type { ProviderId } from '@shared/api';
import { PROVIDER_ENTRIES } from '@shared/providers';
import type { ProviderPages } from '@shared/providers/types';

export interface ProviderGuide extends ProviderPages {
  provider: ProviderId;
}

export const PROVIDER_GUIDES: ProviderGuide[] = PROVIDER_ENTRIES.flatMap((entry) =>
  entry.pages ? [{ provider: entry.id, ...entry.pages }] : [],
);

export function providerGuide(provider: string | undefined): ProviderGuide | undefined {
  return PROVIDER_GUIDES.find((guide) => guide.provider === provider);
}
