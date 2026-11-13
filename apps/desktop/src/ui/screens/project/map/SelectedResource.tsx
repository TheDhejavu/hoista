/** The card beside the map for the resource that is selected. */
import { useState } from 'react';
import { useProject, useLinkResources, useResource } from '@ui/api/projects';
import { Select } from '@ui/components/Select';
import { openExternal } from '@ui/api/system';
import { serviceName } from '@shared/providers';
import { Button, ButtonLink } from '@ui/components/Button';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { statusLook } from '@ui/lib/status';
import { cx } from '@ui/lib/cx';
import { resourceKindLabel } from '../resourceKinds';
import { RemoveFromProject } from '../RemoveFromProject';
import { DeleteAtProvider } from '../DeleteAtProvider';
import { RowMenu } from '@ui/components/RowMenu';
import { useCan } from '@ui/api/providers';
import styles from './SelectedResource.module.css';

export function SelectedResource({
  projectId,
  resourceId,
  onRemoved,
  unlinked,
}: {
  projectId: string;
  resourceId: string;
  /** After the resource is taken out of the app. */
  onRemoved?: () => void;
  /**
   * The project's services, when nothing on the map connects to this resource
   * yet: then the card asks which of them uses it.
   */
  unlinked?: { id: string; name: string }[];
}) {
  const link = useLinkResources(projectId);
  const detail = useResource(resourceId).data;
  const project = useProject(projectId).data;
  const can = useCan();
  const [dialog, setDialog] = useState<'remove' | 'delete' | null>(null);
  if (!detail) return <aside className={styles.card} aria-label="Selected resource" />;

  const { resource } = detail;
  const { label, tone } = statusLook(resource.status);
  const provider = serviceName(resource.provider);
  const kind = resourceKindLabel(resource.kind);
  // "Postgres database", but "Service", not "Service service".
  const kindText =
    resource.product.toLowerCase() === kind.toLowerCase()
      ? resource.product
      : `${resource.product} ${kind.toLowerCase()}`;
  // The provider, account and region; the raw ids live on the resource page.
  const where = detail.summary
    .split(' · ')
    .filter((value) => value !== resource.product)
    .join(' · ');
  const today = `on ${new Date().toISOString().slice(0, 10)}`;
  const facts: [string, string][] = [
    ['Kind', kindText],
    ['Provider', where || provider],
    ...detail.stats
      .slice(0, 2)
      .map((stat): [string, string] => [
        stat.label,
        stat.note === today ? `${stat.value} today` : `${stat.value} ${stat.note ?? ''}`.trim(),
      ]),
  ];
  if (detail.used_by.length > 0) {
    facts.push([
      'Used by',
      detail.used_by.map((user) => `${user.name} via ${user.via}`).join(', '),
    ]);
  }

  return (
    <aside className={styles.card} aria-label="Selected resource">
      <div className={styles.header}>
        <span className={styles.icon}>
          <ServiceIcon id={resource.provider} size={16} />
        </span>
        <span className={styles.name}>{resource.name}</span>
        <span className={cx(styles.pill, resource.status === 'failing' && styles.pillBad)}>
          {tone !== 'hollow' && <span className={cx(styles.dot, styles[tone])} />}
          {label}
        </span>
        {project?.project.mode === 'watching' && (
          <RowMenu
            label={`More for ${resource.name}`}
            items={[
              {
                label: 'Remove from project',
                onSelect: () => {
                  setDialog('remove');
                },
              },
              ...(can(resource.provider, 'delete')
                ? [
                    {
                      label: `Delete on ${provider}`,
                      danger: true,
                      onSelect: () => {
                        setDialog('delete');
                      },
                    },
                  ]
                : []),
            ]}
          />
        )}
      </div>
      <dl className={styles.facts}>
        {facts.map(([term, value]) => (
          <div key={term} className={styles.fact}>
            <dt>{term}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
      {unlinked && unlinked.length > 0 && (
        <div className={styles.ask}>
          <span className={styles.askTitle}>Which service uses this?</span>
          <span className={styles.askWhy}>
            Nothing {provider} reports shows what uses it yet. Pick the service and Hoista draws the
            line. Nothing changes on {provider}.
          </span>
          <Select<string | null>
            large
            wide
            label="Used by"
            value={null}
            options={[
              { value: null, label: link.isPending ? 'Linking' : 'Choose a service' },
              ...unlinked.map((service) => ({ value: service.id, label: service.name })),
            ]}
            onChange={(from) => {
              if (from) link.mutate({ from, to: resource.id });
            }}
          />
        </div>
      )}
      <div className={styles.actions}>
        <ButtonLink to={`/projects/${projectId}/resources/${resource.id}`} variant="accent" large>
          Open resource
        </ButtonLink>
        <Button
          large
          onClick={() => {
            void openExternal(detail.console_url);
          }}
        >
          Open in {provider} ↗
        </Button>
        {project?.project.mode === 'watching' && (
          <>
            <RemoveFromProject
              resource={resource}
              projectName={project.project.name}
              last={project.resources.length === 1}
              opened={dialog === 'remove'}
              onClosed={() => {
                setDialog(null);
              }}
              onRemoved={onRemoved}
            />
            <DeleteAtProvider
              resource={resource}
              last={project.resources.length === 1}
              opened={dialog === 'delete'}
              onClosed={() => {
                setDialog(null);
              }}
            />
          </>
        )}
      </div>
    </aside>
  );
}
