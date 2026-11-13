/**
 * What each provider's engine adapter can do, read once from `GET /providers`.
 * The adapter decides what's offered; nothing here checks a provider by name.
 */
import type { Ability, ProviderAbilities, ProviderLinks, ProviderId } from '@shared/api';
import type { Call } from './call';

export interface Abilities {
  /** Providers the engine has an adapter for. */
  providers: ReadonlySet<string>;
  can: (provider: string, ability: Ability) => boolean;
  /** The provider's pages for making an account and a token. */
  links: (provider: string) => ProviderLinks | undefined;
}

const NONE: Abilities = { providers: new Set(), can: () => false, links: () => undefined };

export function fromProviders(answer: ProviderAbilities): Abilities {
  const byProvider = new Map(answer.items.map((item) => [item.provider, item]));
  return {
    providers: new Set(byProvider.keys()),
    can: (provider, ability) =>
      byProvider.get(provider as ProviderId)?.capabilities[ability] === true,
    links: (provider) => byProvider.get(provider as ProviderId)?.links,
  };
}

/** Asks for the adapters; `strict` passes on a failed read instead of answering with none. */
export type ReadAbilities = (options?: { strict?: boolean }) => Promise<Abilities>;

/**
 * Reads the engine's adapters once and keeps them; a failed read is tried
 * again next time. A failed read counts as no adapters, so lists still show,
 * unless the caller asks `strict`: then the engine's own error comes back,
 * rather than claiming a provider isn't supported.
 */
export function abilitiesOf(call: Call): ReadAbilities {
  let kept: Promise<Abilities> | null = null;
  return async ({ strict = false } = {}) => {
    kept ??= call<ProviderAbilities>('GET', '/providers').then(fromProviders);
    try {
      return await kept;
    } catch (error) {
      kept = null;
      if (strict) throw error;
      return NONE;
    }
  };
}
