/** A resource's metrics as its provider measures them, as charts. */
import { useQuery } from '@tanstack/react-query';
import type { ProviderMetrics } from '@engine-api';
import { apiPath, queryString, request } from './request';
import { MINUTE_MS } from '@shared/time';

export function useProviderMetrics(
  resourceId: string | null,
  window: { from: string; to: string; step: number },
) {
  return useQuery({
    queryKey: ['provider-metrics', resourceId, window.from, window.to, window.step],
    // A new time range keeps this resource's chart up until the new points arrive.
    placeholderData: (previous, previousQuery) =>
      previousQuery?.queryKey[1] === resourceId ? previous : undefined,
    enabled: resourceId !== null,
    staleTime: MINUTE_MS,
    retry: false,
    queryFn: () =>
      request<ProviderMetrics>(
        'GET',
        `${apiPath('resources', resourceId ?? '', 'metrics')}${queryString({
          from: window.from,
          to: window.to,
          step: String(window.step),
        })}`,
      ),
  });
}
