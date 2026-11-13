import { useEffect, useMemo, useRef, useState } from 'react';
import type {
  FoundOnProviders,
  ProviderProject,
  ProviderProjectResource,
  ProviderId,
} from '@shared/api';
import {
  useAddToProject,
  useFoundOnProviders,
  useImportProject,
  useStopWatchingResource,
} from '@ui/api/imports';
import { ALL_ENVIRONMENTS, useCurrentEnvironment } from '@ui/api/environments';
import { useQueryClient } from '@tanstack/react-query';
import { projectKeys } from '@ui/api/keys';
import { useProjects } from '@ui/api/projects';

/**
 * What connecting found, as the provider's projects with everything in them:
 * every provider's, or only `provider`'s (one, or a list).
 */
export function projectsFromFound(
  found: FoundOnProviders,
  provider?: ProviderId | readonly ProviderId[],
): ProviderProject[] {
  const only = provider === undefined ? null : ([] as ProviderId[]).concat(provider);
  const projects = new Map<string, ProviderProject>();
  for (const resource of found.resources) {
    if (only && !only.includes(resource.provider)) continue;
    const key = resource.group ? `${resource.account.id}:${resource.group}` : resource.id;
    let project = projects.get(key);
    if (!project) {
      project = {
        key,
        provider: resource.provider,
        account: resource.account,
        name: resource.group ?? resource.name,
        project: resource.join
          ? { id: resource.join.project_id, name: resource.join.project_name }
          : null,
        resources: [],
      };
      projects.set(key, project);
    }
    project.resources.push({
      id: resource.id,
      name: resource.name,
      kind: resource.kind,
      url: resource.url,
      project_id: null,
    });
  }
  return [...projects.values()];
}

/**
 * What connecting found, read once, fresh, when `active` turns on, and then
 * held still: adding a project, or a background read of a provider, doesn't
 * swap the list out from under you. Only a read finished after it turned on
 * counts, so a copy from before an account was connected never shows as
 * "nothing". `projects` is null until then: still looking.
 */
export function useFoundSnapshot(
  provider: ProviderId | readonly ProviderId[] | undefined,
  active = true,
): { projects: ProviderProject[] | null; error: Error | null } {
  const found = useFoundOnProviders();
  const { refetch } = found;
  const [since, setSince] = useState<number | null>(null);
  if (active && since === null) setSince(found.dataUpdatedAt);
  useEffect(() => {
    if (since !== null) void refetch();
  }, [since, refetch]);
  const [snapshot, setSnapshot] = useState<ProviderProject[] | null>(null);
  const fresh = since !== null && found.dataUpdatedAt > since;
  if (snapshot === null && fresh && found.data && !found.isFetching) {
    setSnapshot(projectsFromFound(found.data, provider));
  }
  return { projects: snapshot, error: found.error };
}

/** Where something is imported to: a Hoista project there is, or a new one by name. */
export type Target = { kind: 'existing'; id: string; name: string } | { kind: 'new'; name: string };

/** What saving does, worked out from the table. */
interface Plan {
  create: { name: string; resource_ids: string[]; environment_id: string }[];
  add: { projectId: string; resourceIds: string[] }[];
  /** Taken out of the project they're in: left out, or on their way to another. */
  remove: string[];
  rename: { projectId: string; name: string }[];
}

export interface Picker {
  projects: ProviderProject[];
  workspaces: { id: string; label: string }[];
  workspace: string;
  setWorkspace: (id: string) => void;
  ticked: Set<string>;
  toggle: (ids: string[], on: boolean) => void;
  /** Where a provider project goes as a whole. */
  groupTarget: (project: ProviderProject) => Target;
  setGroupTarget: (project: ProviderProject, target: Target) => void;
  /** Where one resource goes. */
  rowTarget: (project: ProviderProject, resource: ProviderProjectResource) => Target;
  /** Whether a resource was moved apart from its group. */
  movedApart: (resourceId: string) => boolean;
  moveResource: (resourceId: string, target: Target | null) => void;
  /** Hoista's projects in the chosen environment, to import into. */
  existing: { id: string; name: string }[];
  /** The Hoista environment new projects go into, and existing ones are picked from. */
  environment: string;
  setEnvironment: (id: string) => void;
  plan: Plan;
  /** What saving does, in a line. */
  summary: string;
  /** New projects saving makes from the shown workspace. */
  projectsInWorkspace: number;
  changed: boolean;
}

export const sameName = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();

export function targetKey(target: Target): string {
  return target.kind === 'existing' ? `existing:${target.id}` : `new:${target.name}`;
}

/**
 * The picker's state. `start` is what begins ticked: everything when adding
 * what was found, what is already in a project when managing.
 */
export function usePicker(
  projects: ProviderProject[],
  start: 'everything' | 'in-projects',
  firstWorkspace?: string,
): Picker {
  const shown = useCurrentEnvironment().id;
  const [chosenEnvironment, setEnvironment] = useState<string | null>(null);
  const environment = chosenEnvironment ?? shown;
  // Every project, so a resource already in one anywhere reads as imported;
  // only the chosen environment's are offered to import into.
  const projectList = useProjects({ environment: ALL_ENVIRONMENTS });
  const all = useMemo(
    () =>
      (projectList.data?.items ?? []).map((item) => ({
        id: item.id,
        name: item.name,
        environment: item.environment_id,
      })),
    [projectList.data],
  );
  const existing = useMemo(
    () =>
      all
        .filter((item) => item.environment === environment)
        .map((item) => ({ id: item.id, name: item.name })),
    [all, environment],
  );
  const workspaces = useMemo(() => {
    const seen = new Map<string, string>();
    for (const project of projects) seen.set(project.account.id, project.account.label);
    return [...seen].map(([id, label]) => ({ id, label }));
  }, [projects]);
  const initial = useMemo(
    () =>
      new Set(
        projects.flatMap((project) =>
          project.resources
            .filter((resource) => start === 'everything' || resource.project_id !== null)
            .map((resource) => resource.id),
        ),
      ),
    [projects, start],
  );
  const [ticked, setTicked] = useState<Set<string>>(initial);
  const [groups, setGroups] = useState<Record<string, Target>>({});
  const [rows, setRows] = useState<Record<string, Target>>({});
  const [chosenWorkspace, setWorkspace] = useState<string | undefined>(firstWorkspace);

  // New data (after saving, or a sign-in finishing) starts from what's true now.
  const seen = useRef(initial);
  useEffect(() => {
    if (seen.current === initial) return;
    seen.current = initial;
    setTicked(initial);
    setGroups({});
    setRows({});
  }, [initial]);

  const workspace =
    chosenWorkspace && workspaces.some((item) => item.id === chosenWorkspace)
      ? chosenWorkspace
      : (workspaces[0]?.id ?? '');

  const known = (id: string | null | undefined) => {
    const found = id ? all.find((item) => item.id === id) : undefined;
    return found ? { id: found.id, name: found.name } : null;
  };

  /** Where it is now, else the Hoista project of its name, else a new one of its name. */
  const defaultTarget = (project: ProviderProject): Target => {
    const held =
      known(project.project?.id) ??
      known(project.resources.find((resource) => resource.project_id)?.project_id);
    if (held) return { kind: 'existing', ...held };
    const named =
      existing.find((item) => sameName(item.name, project.project?.name ?? project.name)) ?? null;
    if (named) return { kind: 'existing', ...named };
    if (project.project) return { kind: 'existing', ...project.project };
    return { kind: 'new', name: project.name };
  };
  const groupTarget = (project: ProviderProject) => groups[project.key] ?? defaultTarget(project);
  const rowTarget = (project: ProviderProject, resource: ProviderProjectResource): Target => {
    const own = rows[resource.id];
    if (own) return own;
    // A resource already in a project stays there unless its group is sent elsewhere.
    const held = groups[project.key] ? null : known(resource.project_id);
    return held ? { kind: 'existing', ...held } : groupTarget(project);
  };

  const plan: Plan = { create: [], add: [], remove: [], rename: [] };
  const creates = new Map<string, string[]>();
  const adds = new Map<string, string[]>();
  for (const project of projects) {
    for (const resource of project.resources) {
      const current = resource.project_id;
      if (!ticked.has(resource.id)) {
        if (current) plan.remove.push(resource.id);
        continue;
      }
      const target = rowTarget(project, resource);
      if (target.kind === 'existing') {
        if (current === target.id) continue;
        if (current) plan.remove.push(resource.id);
        adds.set(target.id, [...(adds.get(target.id) ?? []), resource.id]);
      } else {
        if (current) plan.remove.push(resource.id);
        creates.set(target.name, [...(creates.get(target.name) ?? []), resource.id]);
      }
    }
  }
  for (const [name, ids] of creates)
    plan.create.push({ name, resource_ids: ids, environment_id: environment });
  for (const [id, ids] of adds) plan.add.push({ projectId: id, resourceIds: ids });

  const inWorkspace = projects.filter((project) => project.account.id === workspace);
  const projectsInWorkspace = new Set(
    inWorkspace.flatMap((project) =>
      project.resources
        .filter((resource) => ticked.has(resource.id))
        .map((resource) => rowTarget(project, resource))
        .flatMap((target) => (target.kind === 'new' ? [target.name] : [])),
    ),
  ).size;

  const moved = plan.add.reduce((sum, item) => sum + item.resourceIds.length, 0);
  const going = new Set([
    ...plan.add.flatMap((item) => item.resourceIds),
    ...plan.create.flatMap((item) => item.resource_ids),
  ]);
  const takenOut = plan.remove.filter((id) => !going.has(id)).length;
  const plural = (count: number, one: string, many: string) =>
    `${String(count)} ${count === 1 ? one : many}`;
  const summary = [
    plan.create.length > 0
      ? `${plural(plan.create.length, 'new project', 'new projects')} with ${plural(
          plan.create.reduce((sum, item) => sum + item.resource_ids.length, 0),
          'resource',
          'resources',
        )}`
      : '',
    moved > 0
      ? `${plural(moved, 'resource', 'resources')} into ${plural(plan.add.length, 'existing project', 'existing projects')}`
      : '',
    takenOut > 0 ? `${String(takenOut)} taken out` : '',
  ]
    .filter(Boolean)
    .join(' · ');

  return {
    projects,
    workspaces,
    workspace,
    setWorkspace,
    ticked,
    toggle: (ids, on) => {
      setTicked((current) => {
        const next = new Set(current);
        for (const id of ids) {
          if (on) next.add(id);
          else next.delete(id);
        }
        return next;
      });
    },
    groupTarget,
    setGroupTarget: (project, target) => {
      setGroups((current) => ({ ...current, [project.key]: target }));
    },
    rowTarget,
    movedApart: (id) => id in rows,
    moveResource: (id, target) => {
      setRows((current) => {
        if (target) return { ...current, [id]: target };
        return Object.fromEntries(Object.entries(current).filter(([key]) => key !== id));
      });
      if (target) setTicked((current) => new Set(current).add(id));
    },
    existing,
    environment,
    setEnvironment,
    plan,
    summary,
    projectsInWorkspace,
    changed: plan.create.length + plan.add.length + plan.remove.length > 0,
  };
}

/** Carries out a plan: take out first, then new projects, then additions. Nothing changes at the provider. */
export function useApplyPicker() {
  const importProject = useImportProject();
  const addTo = useAddToProject();
  const stopWatching = useStopWatchingResource();
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<Error | null>(null);

  async function apply(plan: Plan): Promise<boolean> {
    setBusy(true);
    setError(null);
    try {
      for (const id of plan.remove) await stopWatching.mutateAsync(id);
      for (const project of plan.create) await importProject.mutateAsync(project);
      for (const item of plan.add) await addTo.mutateAsync(item);
      // The lists catch up in the background; the import itself is done.
      void queryClient.invalidateQueries({ queryKey: projectKeys.all });
      return true;
    } catch (caught) {
      setError(caught instanceof Error ? caught : new Error('Something went wrong.'));
      return false;
    } finally {
      setBusy(false);
    }
  }

  return { apply, busy, error };
}
