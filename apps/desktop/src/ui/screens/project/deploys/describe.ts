/**
 * Plain-English descriptions of deploys, shared by the Deploys tab and the
 * roll back dialog.
 */
import type { Deploy } from '@shared/api';

/**
 * The change in a few words: the first line of the commit message (its
 * subject, as GitHub shows it) without markdown links, or the source if
 * there is no message.
 */
export function changeTitle(deploy: Deploy): string {
  const message = deploy.source.message?.trim();
  if (!message) return deploy.source.ref;
  const subject = message.split(/\r?\n/, 1)[0] ?? message;
  return subject.replace(/\[([^\]]*)\]\([^)]*\)/g, '$1').trim() || deploy.source.ref;
}

/**
 * Only versions that went live can be rolled back to, not the one live now,
 * and not one the provider says it can't go back to.
 */
export function canRollBackTo(deploy: Deploy): boolean {
  return deploy.status === 'live' && !deploy.current && deploy.can_roll_back !== false;
}

/** The version a plain "Roll back" goes to: the newest live one before the current. */
export function previousVersion(deploys: Deploy[]): Deploy | null {
  const currentIndex = deploys.findIndex((deploy) => deploy.current);
  return deploys.slice(currentIndex + 1).find(canRollBackTo) ?? null;
}

/** Versions newer than `target` that went live, newest first. Their changes disappear on roll back. */
export function changesUndoneBy(deploys: Deploy[], target: Deploy): Deploy[] {
  const targetIndex = deploys.indexOf(target);
  return deploys.slice(0, targetIndex).filter((deploy) => deploy.status === 'live');
}

/**
 * The deploys of the one resource whose newest deploy failed, newest first,
 * or null when none failed. A watched project's deploys are per resource (each
 * service deploys on its own at the provider); a managed project deploys as a whole.
 */
export function failingDeploys(deploys: Deploy[]): Deploy[] | null {
  const groups = new Map<string, Deploy[]>();
  for (const deploy of deploys) {
    const key = deploy.resource_id ?? '';
    groups.set(key, [...(groups.get(key) ?? []), deploy]);
  }
  const failing = [...groups.values()]
    .filter((group) => group[0]?.status === 'failed')
    .sort((a, b) => ((a[0]?.created_at ?? '') < (b[0]?.created_at ?? '') ? 1 : -1));
  return failing[0] ?? null;
}
