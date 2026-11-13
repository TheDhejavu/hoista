/**
 * "Add resources" on a project's overview: pick what connecting your providers
 * found that isn't in a project yet, and add it here. Nothing changes at the
 * provider; Hoista starts watching what you pick.
 */
import { useState } from 'react';
import { serviceName } from '@shared/providers';
import { useAddToProject, useFoundOnProviders } from '@ui/api/imports';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { QueryState } from '@ui/components/QueryState';
import { SkeletonRows } from '@ui/components/Skeleton';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { resourceKindLabel } from '../resourceKinds';
import styles from './AddResources.module.css';

export function AddResources({ projectId }: { projectId: string }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button
        onClick={() => {
          setOpen(true);
        }}
      >
        Add resources
      </Button>
      {open && (
        <AddDialog
          projectId={projectId}
          onClose={() => {
            setOpen(false);
          }}
        />
      )}
    </>
  );
}

function AddDialog({ projectId, onClose }: { projectId: string; onClose: () => void }) {
  const found = useFoundOnProviders();
  const add = useAddToProject();
  const [ticked, setTicked] = useState<Set<string>>(new Set());
  const count = ticked.size;

  return (
    <Dialog
      title="Add resources to this project"
      eyebrow="Found on your connected providers and not in a project yet. Nothing changes at the provider."
      onClose={onClose}
      actions={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="accent"
            disabled={count === 0}
            busy={add.isPending}
            onClick={() => {
              add.mutate(
                { projectId, resourceIds: [...ticked] },
                {
                  onSuccess: onClose,
                },
              );
            }}
          >
            {count === 0 ? 'Add' : `Add ${count}`}
          </Button>
        </>
      }
    >
      <QueryState query={found} skeleton={<SkeletonRows count={4} columns={3} />}>
        {(data) =>
          data.resources.length === 0 ? (
            <p className={styles.empty}>
              No resources outside a project. To move one between projects, use Move to another
              project in its ⋮ menu.
            </p>
          ) : (
            <ul className={styles.list}>
              {data.resources.map((resource) => {
                const id = `add-${resource.id}`;
                return (
                  <li key={resource.id}>
                    <label htmlFor={id} className={styles.row}>
                      <input
                        id={id}
                        type="checkbox"
                        checked={ticked.has(resource.id)}
                        onChange={(event) => {
                          const next = new Set(ticked);
                          if (event.target.checked) next.add(resource.id);
                          else next.delete(resource.id);
                          setTicked(next);
                        }}
                      />
                      <span className={styles.logo} title={serviceName(resource.provider)}>
                        <ServiceIcon id={resource.provider} size={14} />
                      </span>
                      <span className={styles.named}>
                        <span className={styles.name}>{resource.name}</span>
                        <span className={styles.meta}>
                          {[
                            resourceKindLabel(resource.kind),
                            resource.group,
                            resource.account.label,
                          ]
                            .filter(Boolean)
                            .join(' · ')}
                        </span>
                      </span>
                    </label>
                  </li>
                );
              })}
            </ul>
          )
        }
      </QueryState>
      {add.error && <ErrorMessage error={add.error} />}
    </Dialog>
  );
}
