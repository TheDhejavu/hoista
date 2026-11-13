/**
 * Whether something about a project belongs in the environment the window
 * shows. Deploys, agent activity and problems follow the environment, like the
 * Projects list. Things with no project (an account, an agent connecting) and
 * projects Hoista doesn't know yet are always shown.
 */
import { ALL_ENVIRONMENTS, useCurrentEnvironment } from '@ui/api/environments';
import { useProjects } from '@ui/api/projects';

export function useInEnvironment(): (projectId: string | null | undefined) => boolean {
  const current = useCurrentEnvironment().id;
  const everywhere = useProjects({ environment: ALL_ENVIRONMENTS }).data?.items ?? [];
  const homes = new Map(everywhere.map((project) => [project.id, project.environment_id]));
  return (projectId) => {
    if (!projectId) return true;
    const home = homes.get(projectId);
    return home === undefined || home === current;
  };
}
