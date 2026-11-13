/**
 * What the setup steps share: the providers picked in step 1 (kept in the
 * address, `?providers=railway,cloudflare`, so Back and a reload keep them),
 * how many projects were imported, and finishing or skipping setup.
 */
import { useOutletContext, useSearchParams } from 'react-router-dom';
import type { ProviderId } from '@shared/api';
import { PROVIDERS } from '@ui/lib/providers';

export type SetupStepId = 'providers' | 'connect' | 'projects' | 'ai';

export interface SetupContext {
  /** New Hoista projects made in the Projects step, once it's done. */
  imported: number | null;
  setImported: (count: number) => void;
  /** Saves the profile and opens Projects, with a note saying what was set up. */
  finish: (agent: string | null) => void;
  /** Saves the profile and opens Projects, setting up nothing more. */
  skip: () => void;
  /** Saving the profile is under way. */
  finishing: boolean;
  finishError: Error | null;
}

export function useSetup(): SetupContext {
  return useOutletContext<SetupContext>();
}

/** The providers picked in step 1, in the order the list shows them. */
export function usePicked(): ProviderId[] {
  const [params] = useSearchParams();
  const asked = new Set((params.get('providers') ?? '').split(','));
  return PROVIDERS.map((provider) => provider.id).filter((id) => asked.has(id));
}

/** The address of a step, carrying the picked providers along. */
export function stepPath(step: SetupStepId, picked: readonly ProviderId[]): string {
  return picked.length > 0 ? `/setup/${step}?providers=${picked.join(',')}` : `/setup/${step}`;
}
