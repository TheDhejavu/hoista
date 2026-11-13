/**
 * Project names by id, for screens that only hold a project id (activity, deploys,
 * prompts). Falls back to the id until the app list has loaded, and for apps
 * that no longer exist.
 */
import { useProjects } from '@ui/api/projects';
import { ALL_ENVIRONMENTS } from '@ui/api/environments';

export function useProjectNames(): (id: string | null | undefined) => string {
  const projects = useProjects({ environment: ALL_ENVIRONMENTS }).data?.items ?? [];
  const names = new Map(projects.map((project) => [project.id, project.name]));
  return (id) => (id ? (names.get(id) ?? id) : '');
}
