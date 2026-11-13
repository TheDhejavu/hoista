import type { ServiceId } from '@shared/api';
import { serviceName } from '@shared/providers';

/** The providers that tell a project apart from one with the same name, or undefined. */
export function projectNote(
  project: { id: string; name: string; providers: readonly ServiceId[] },
  projects: readonly { id: string; name: string }[],
): string | undefined {
  const twin = projects.some((other) => other.id !== project.id && other.name === project.name);
  if (!twin || project.providers.length === 0) return undefined;
  return project.providers.map((id) => serviceName(id)).join(', ');
}
