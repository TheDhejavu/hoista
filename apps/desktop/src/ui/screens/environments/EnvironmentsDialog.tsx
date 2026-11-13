/**
 * Hoista's environments: separate spaces for projects, such as default and
 * production. Add, rename and remove them here; a project moves between them
 * from its settings or the import picker. Nothing changes at any provider.
 */
import { useState } from 'react';
import type { Environment } from '@shared/api';
import {
  ALL_ENVIRONMENTS,
  DEFAULT_ENVIRONMENT,
  useCreateEnvironment,
  useCurrentEnvironment,
  useEnvironments,
  useRemoveEnvironment,
  useRenameEnvironment,
} from '@ui/api/environments';
import { useProjects } from '@ui/api/projects';
import { Button } from '@ui/components/Button';
import { Dialog, DialogText } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { TextField } from '@ui/components/Field';
import { QueryState } from '@ui/components/QueryState';
import { SkeletonRows } from '@ui/components/Skeleton';
import { RowMenu } from '@ui/components/RowMenu';
import { showToast } from '@ui/lib/toast';
import styles from './EnvironmentsDialog.module.css';

export function EnvironmentsDialog({ onClose }: { onClose: () => void }) {
  const current = useCurrentEnvironment();
  const environments = useEnvironments();
  const projects = useProjects({ environment: ALL_ENVIRONMENTS });
  const [naming, setNaming] = useState<{ environment: Environment | null } | null>(null);
  const [removing, setRemoving] = useState<Environment | null>(null);
  const count = (id: string) =>
    (projects.data?.items ?? []).filter((project) => project.environment_id === id).length;

  return (
    <Dialog
      title="Environments"
      eyebrow="Separate spaces for your projects"
      onClose={onClose}
      sideAction={
        <Button
          onClick={() => {
            setNaming({ environment: null });
          }}
        >
          New environment
        </Button>
      }
      actions={
        <Button variant="primary" onClick={onClose}>
          Done
        </Button>
      }
    >
      <DialogText>
        Each project lives in one environment. Import into one, move projects between them, and
        limit an agent to some, for example only default. Nothing changes at your providers.
      </DialogText>
      <QueryState query={environments} skeleton={<SkeletonRows count={2} columns={2} />}>
        {() => (
          <ul className={styles.list} aria-label="Environments">
            {current.environments.map((environment) => {
              const projectCount = count(environment.id);
              const isDefault = environment.id === DEFAULT_ENVIRONMENT;
              return (
                <li key={environment.id} className={styles.row}>
                  <span className={styles.name}>
                    {environment.name}
                    {environment.id === current.id && <span className={styles.tag}>Showing</span>}
                  </span>
                  <span className={styles.meta}>
                    {projectCount === 1 ? '1 project' : `${String(projectCount)} projects`}
                  </span>
                  <span className={styles.actions}>
                    {environment.id !== current.id && (
                      <Button
                        onClick={() => {
                          current.set(environment.id);
                          showToast(`Showing ${environment.name}`);
                        }}
                      >
                        Show
                      </Button>
                    )}
                    <RowMenu
                      label={`More for ${environment.name}`}
                      items={[
                        {
                          label: 'Rename',
                          onSelect: () => {
                            setNaming({ environment });
                          },
                        },
                        {
                          label: 'Remove',
                          danger: true,
                          disabled: isDefault,
                          onSelect: () => {
                            setRemoving(environment);
                          },
                        },
                      ]}
                    />
                  </span>
                </li>
              );
            })}
          </ul>
        )}
      </QueryState>
      {naming && (
        <NameDialog
          environment={naming.environment}
          onClose={() => {
            setNaming(null);
          }}
        />
      )}
      {removing && (
        <RemoveDialog
          environment={removing}
          projects={count(removing.id)}
          onClose={() => {
            setRemoving(null);
          }}
        />
      )}
    </Dialog>
  );
}

/** New environment, or a new name for one. Opens over the list, which stays. */
export function NameDialog({
  environment,
  onClose,
  onCreated,
}: {
  environment: Environment | null;
  onClose: () => void;
  onCreated?: (environment: Environment) => void;
}) {
  const create = useCreateEnvironment();
  const rename = useRenameEnvironment();
  const [name, setName] = useState(environment?.name ?? '');
  const busy = create.isPending || rename.isPending;
  const clean = name.trim().toLowerCase();

  function save(): void {
    if (environment) {
      rename.mutate(
        { id: environment.id, name: clean },
        {
          onSuccess: () => {
            showToast(`Renamed to ${clean}`);
            onClose();
          },
        },
      );
    } else {
      create.mutate(clean, {
        onSuccess: (made) => {
          showToast(`Created ${made.name}`);
          onCreated?.(made);
          onClose();
        },
      });
    }
  }

  return (
    <Dialog
      title={environment ? `Rename ${environment.name}` : 'New environment'}
      eyebrow="Environments"
      onClose={onClose}
      actions={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            busy={busy}
            busyLabel={environment ? 'Renaming' : 'Creating'}
            disabled={!clean || clean === environment?.name}
            onClick={save}
          >
            {environment ? 'Rename' : 'Create environment'}
          </Button>
        </>
      }
    >
      <TextField
        label="Name"
        value={name}
        placeholder="production"
        hint="Lowercase letters, digits and hyphens, such as production or staging."
        onChange={setName}
      />
      {!environment && (
        <DialogText>It starts empty. Import projects into it, or move them in.</DialogText>
      )}
      <ErrorMessage error={create.error ?? rename.error} />
    </Dialog>
  );
}

function RemoveDialog({
  environment,
  projects,
  onClose,
}: {
  environment: Environment;
  projects: number;
  onClose: () => void;
}) {
  const remove = useRemoveEnvironment();
  return (
    <Dialog
      title={`Remove ${environment.name}?`}
      eyebrow="Environments"
      onClose={onClose}
      actions={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="danger"
            busy={remove.isPending}
            busyLabel="Removing"
            disabled={projects > 0}
            onClick={() => {
              remove.mutate(environment.id, {
                onSuccess: () => {
                  showToast(`Removed ${environment.name}`);
                  onClose();
                },
              });
            }}
          >
            Remove
          </Button>
        </>
      }
    >
      <DialogText>
        {projects > 0
          ? `${environment.name} still has ${projects === 1 ? '1 project' : `${String(projects)} projects`}. Move them to another environment first, from each project’s settings.`
          : 'Agents limited to it lose access to it. Nothing changes at your providers.'}
      </DialogText>
      <ErrorMessage error={remove.error} />
    </Dialog>
  );
}
