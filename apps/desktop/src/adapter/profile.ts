/**
 * Profile routes. The daemon stores a name and initials and answers 404
 * before first run; the desktop expects null then, and a suggested name from
 * this computer's account.
 */
import type { ProfileDto } from '@engine-api';
import type { Profile, ProfileUpdate } from '@shared/api';
import { RouteError, type Router } from '@shared/router';
import { orNull, type Call } from './call';

export function addProfileRoutes(router: Router, call: Call, userName: string): void {
  router.on('GET', '/profile', async (): Promise<Profile | null> => {
    const profile = await orNull(call<ProfileDto>('GET', '/profile'));
    return profile ? toProfile(profile) : null;
  });

  router.on('GET', '/profile/suggestion', (): ProfileUpdate => ({
    name: suggestedName(userName),
  }));

  router.on('PUT', '/profile', async ({ body }): Promise<Profile> => {
    const update = readUpdate(body);
    const saved = await call<ProfileDto>('PUT', '/profile', {
      name: update.name,
      initials: initialsOf(update.name),
    });
    return toProfile(saved);
  });
}

function toProfile(profile: ProfileDto): Profile {
  return { name: profile.name, initials: profile.initials };
}

function readUpdate(body: unknown): ProfileUpdate {
  if (typeof body !== 'object' || body === null || !('name' in body)) {
    throw new RouteError('profile.invalid', 'Enter your name.');
  }
  const name = typeof body.name === 'string' ? body.name.trim() : '';
  if (!name) throw new RouteError('profile.invalid', 'Enter your name.');
  return { name };
}

/** `Ada Lovelace` → `AL`; one word gives one letter. At most two letters. */
export function initialsOf(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  const letters = words.length > 1 ? [words[0], words[words.length - 1]] : words;
  return letters
    .map((word) => Array.from(word ?? '')[0] ?? '')
    .join('')
    .toUpperCase();
}

/** The account name on this computer, tidied (`sam.rivera` as `Sam Rivera`); `You` when there is none. */
function suggestedName(account: string): string {
  const words = account
    .split(/[._\-\s]+/)
    .filter(Boolean)
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1));
  return words.join(' ') || 'You';
}
