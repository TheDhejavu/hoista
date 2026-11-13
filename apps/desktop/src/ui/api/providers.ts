/**
 * What each provider's engine adapter can do (`GET /providers`), so screens show
 * only what a provider supports. Nothing in the app checks a provider by name.
 */
import { useQuery } from '@tanstack/react-query';
import type { Ability, ProviderAbilities } from '@shared/api';
import { request } from './request';

const providerKeys = { abilities: ['providers'] as const };

export function useProviderAbilities() {
  return useQuery({
    queryKey: providerKeys.abilities,
    queryFn: () => request<ProviderAbilities>('GET', '/providers'),
    // Adapters don't change while the app runs.
    staleTime: Infinity,
  });
}

/** `can(provider, ability)`: false until the engine has said, and for providers it has no adapter for. */
export function useCan(): (provider: string, ability: Ability) => boolean {
  const items = useProviderAbilities().data?.items;
  return (provider, ability) =>
    items?.find((item) => item.provider === provider)?.capabilities[ability] === true;
}
