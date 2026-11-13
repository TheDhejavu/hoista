import { SECOND_MS } from '@shared/time';

/**
 * How often a screen asks again for something no engine event announces,
 * by how soon the person needs to see the change.
 */
export const POLL_MS = {
  /** Something the person is watching for right now, such as an agent signing in. */
  watching: 2 * SECOND_MS,
  /** State that changes outside Hoista, such as the network address or the MCP link. */
  live: 3 * SECOND_MS,
  /** A check that just failed, asked again soon. */
  retry: 3 * SECOND_MS,
  /** Something that takes a while, such as a new server starting or a remote coming back. */
  slow: 10 * SECOND_MS,
  /** The engine's own health. */
  health: 15 * SECOND_MS,
  /** Background state that rarely changes. */
  idle: 30 * SECOND_MS,
} as const;
