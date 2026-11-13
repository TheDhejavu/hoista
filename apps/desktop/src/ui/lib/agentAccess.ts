/**
 * Wording for agent access levels, shared by the agents and settings screens,
 * the way ChatGPT names its own: View only, Ask for approval, Full access.
 */
import type { AccessLevel } from '@shared/api';
import type { Option } from '@ui/components/Select';

export const ACCESS_CHOICES: Option<AccessLevel>[] = [
  { value: 'look', label: 'View only' },
  { value: 'ask', label: 'Ask for approval' },
  { value: 'full', label: 'Full access' },
];

/** `Ask for approval`. */
export function accessLabel(access: AccessLevel): string {
  return ACCESS_CHOICES.find((option) => option.value === access)?.label ?? access;
}

/** What each level allows, in a few words, shown under the choice. */
export const ACCESS_HINTS: Record<AccessLevel, string> = {
  look: 'Sees your projects, logs, deploys and bills. Can’t change anything',
  ask: 'Asks before deleting, removing or connecting an account',
  full: 'Changes anything, including deleting, without asking',
};

export const ALWAYS_ASKS =
  'With Ask for approval, deleting or removing anything asks you first, in the agent itself.';
