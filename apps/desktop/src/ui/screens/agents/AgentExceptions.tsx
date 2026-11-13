/**
 * Exceptions to an agent's access: a group of resources (services,
 * databases, storage, domains) in every project or one, or a single
 * resource, set to Ask first, View only or No access. They only ever narrow
 * what the agent may do; the strictest one that applies wins.
 */
import { useState } from 'react';
import type { AgentException, ResourceGroup } from '@shared/api';
import { useProject } from '@ui/api/projects';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { RowMenu } from '@ui/components/RowMenu';
import { SelectField, type Option } from '@ui/components/Select';
import { resourceKindLabel } from '../project/resourceKinds';
import styles from './AgentExceptions.module.css';

const GROUPS: Record<ResourceGroup, string> = {
  services: 'Services',
  databases: 'Databases',
  storage: 'Storage',
  domains: 'Domains',
};

const ACCESS: Record<AgentException['access'], string> = {
  ask: 'Ask first',
  look: 'View only',
  hidden: 'No access',
};

const ACCESS_NOTES: Record<AgentException['access'], string> = {
  ask: 'Deleting or removing anything asks you first, even with Full access.',
  look: 'It can see status, logs, metrics and settings, and change nothing.',
  hidden: 'Hidden from it, as if it weren’t there.',
};

type Target = ResourceGroup | 'resource';

const TARGETS: Option<Target | null>[] = [
  { value: null, label: 'None' },
  ...Object.entries(GROUPS).map(([value, label]) => ({
    value: value as ResourceGroup,
    label,
    group: 'A group of resources',
  })),
  { value: 'resource', label: 'One resource', group: 'Just one' },
];

const ACCESS_CHOICES: Option<AgentException['access'] | null>[] = [
  { value: null, label: 'None' },
  ...Object.entries(ACCESS).map(([value, label]) => ({
    value: value as AgentException['access'],
    label,
  })),
];

function same(a: AgentException, b: AgentException): boolean {
  return a.group === b.group && a.project_id === b.project_id && a.resource_id === b.resource_id;
}

/** A project the agent can reach, as the access dialog's choices above leave them. */
export interface Reachable {
  id: string;
  name: string;
}

export function ExceptionsSection({
  exceptions,
  reachable,
  onChange,
}: {
  exceptions: AgentException[];
  /** The projects its environments and projects leave it; exceptions name only these. */
  reachable: Reachable[];
  onChange: (next: AgentException[]) => void;
}) {
  const inReach = (exception: AgentException) =>
    exception.project_id === null ||
    reachable.some((project) => project.id === exception.project_id);
  const [adding, setAdding] = useState(false);
  return (
    <div className={styles.section}>
      <div className={styles.head}>
        <Button
          onClick={() => {
            setAdding(true);
          }}
        >
          Add exception
        </Button>
      </div>
      {exceptions.length === 0 ? (
        <p className={styles.empty}>None. Its access applies to every resource it can reach.</p>
      ) : (
        <ul className={styles.list} aria-label="Exceptions">
          {exceptions.map((exception) => (
            <li
              key={`${exception.group ?? ''}:${exception.project_id ?? ''}:${exception.resource_id ?? ''}`}
              className={styles.row}
            >
              <ExceptionTarget exception={exception} />
              {inReach(exception) ? (
                <span className={styles.access}>{ACCESS[exception.access]}</span>
              ) : (
                <span
                  className={styles.outside}
                  title="Its project is outside what it can reach above"
                >
                  Outside its reach
                </span>
              )}
              <RowMenu
                label="More for this exception"
                items={[
                  {
                    label: 'Remove',
                    danger: true,
                    onSelect: () => {
                      onChange(exceptions.filter((other) => !same(other, exception)));
                    },
                  },
                ]}
              />
            </li>
          ))}
        </ul>
      )}
      {adding && (
        <AddExceptionDialog
          reachable={reachable}
          onClose={() => {
            setAdding(false);
          }}
          onAdd={(exception) => {
            onChange([...exceptions.filter((other) => !same(other, exception)), exception]);
            setAdding(false);
          }}
        />
      )}
    </div>
  );
}

/** What an exception covers, in words: "Databases · every project". */
function ExceptionTarget({ exception }: { exception: AgentException }) {
  const project = useProject(exception.project_id ?? '', {
    enabled: exception.project_id !== null,
  }).data;
  const projectName = project?.project.name ?? exception.project_id;
  if (exception.resource_id !== null) {
    const resource = project?.resources.find((item) => item.id === exception.resource_id);
    return (
      <span className={styles.target}>
        <b>{resource?.name ?? exception.resource_id}</b>
        {resource && <span className={styles.where}>{resourceKindLabel(resource.kind)}</span>}
        <span className={styles.where}>{projectName ? `in ${projectName}` : 'one resource'}</span>
      </span>
    );
  }
  return (
    <span className={styles.target}>
      <b>{exception.group ? GROUPS[exception.group] : ''}</b>
      <span className={styles.where}>
        {exception.project_id === null
          ? 'in every project it can reach'
          : `in ${projectName ?? ''}`}
      </span>
    </span>
  );
}

/** Opened over the access dialog, which stays open underneath. */
function AddExceptionDialog({
  reachable: projects,
  onClose,
  onAdd,
}: {
  reachable: Reachable[];
  onClose: () => void;
  onAdd: (exception: AgentException) => void;
}) {
  const [target, setTarget] = useState<Target | null>(null);
  const [projectId, setProjectId] = useState<string | null>(null);
  const [resourceId, setResourceId] = useState<string | null>(null);
  const [access, setAccess] = useState<AgentException['access'] | null>(null);
  const one = target === 'resource';
  const detail = useProject(projectId ?? '', { enabled: one && projectId !== null });
  const resources = detail.data?.resources ?? [];

  const projectChoices: Option<string | null>[] = [
    one ? { value: null, label: 'None' } : { value: null, label: 'Every project it can reach' },
    ...projects.map((project) => ({ value: project.id, label: project.name })),
  ];
  const resourceChoices: Option<string | null>[] = [
    { value: null, label: detail.isLoading ? 'Loading resources' : 'None' },
    ...resources.map((resource) => ({
      value: resource.id,
      label: resource.name,
      note: resourceKindLabel(resource.kind),
    })),
  ];
  const ready =
    target !== null && access !== null && (!one || (projectId !== null && resourceId !== null));

  return (
    <Dialog
      title="Add exception"
      eyebrow="Agent access"
      onClose={onClose}
      actions={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={!ready}
            onClick={() => {
              if (!ready) return;
              onAdd({
                group: one ? null : target,
                project_id: projectId,
                resource_id: one ? resourceId : null,
                access,
              });
            }}
          >
            Add
          </Button>
        </>
      }
    >
      <div className={styles.fields}>
        <SelectField
          label="Applies to"
          value={target}
          options={TARGETS}
          onChange={(next) => {
            setTarget(next);
            setResourceId(null);
            if (next === 'resource' && projectId === null) setProjectId(null);
          }}
        />
        <SelectField
          label={one ? 'Project' : 'Where'}
          value={projectId}
          options={projectChoices}
          onChange={(next) => {
            setProjectId(next);
            setResourceId(null);
          }}
        />
        {one && projectId !== null && (
          <SelectField
            label="Resource"
            value={resourceId}
            options={resourceChoices}
            onChange={setResourceId}
          />
        )}
        <SelectField
          label="Access there"
          value={access}
          options={ACCESS_CHOICES}
          onChange={setAccess}
        />
        <p className={styles.hint}>
          {access
            ? ACCESS_NOTES[access]
            : 'Pick how far it narrows the agent’s access for what you chose.'}
        </p>
      </div>
    </Dialog>
  );
}
