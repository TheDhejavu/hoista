/**
 * A resource's Settings tab: which project it is in, with a dropdown to move
 * it to another (nothing changes at the provider), and taking it out of Hoista or
 * deleting it at its provider.
 */
import { useNavigate } from 'react-router-dom';
import type { Resource } from '@shared/api';
import { serviceName } from '@shared/providers';
import { useMoveResources } from '@ui/api/imports';
import { useProject, useProjects } from '@ui/api/projects';
import { useCan } from '@ui/api/providers';
import { useServerAgents, useSetServerAgents } from '@ui/api/servers';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { Select } from '@ui/components/Select';
import { Skeleton } from '@ui/components/Skeleton';
import { DeleteAtProvider } from '../DeleteAtProvider';
import { RemoveFromProject } from '../RemoveFromProject';
import { Section } from './ResourceSection';
import styles from './ResourceScreen.module.css';

export function ResourceSettings({
  projectId,
  resource,
}: {
  projectId: string;
  resource: Resource;
}) {
  const navigate = useNavigate();
  const can = useCan();
  const project = useProject(projectId).data;
  const projects = useProjects().data?.items ?? [];
  const move = useMoveResources();
  const provider = serviceName(resource.provider);
  const watching = project?.project.mode === 'watching';
  const last = (project?.resources.length ?? 1) <= 1;
  const options = projects
    .map((item) => ({ value: item.id, label: item.name }))
    .sort((a, b) => a.label.localeCompare(b.label));

  return (
    <div className={styles.settings}>
      <Section title="Project">
        <div className={styles.settingRow}>
          <div className={styles.settingText}>
            <b>Move to another project</b>
            <span>
              {move.isPending
                ? 'Moving'
                : `It keeps running on ${provider} as it is; only where Hoista lists it changes.`}
            </span>
          </div>
          <Select
            label="Project"
            value={projectId}
            options={options}
            disabled={move.isPending || options.length < 2}
            onChange={(target) => {
              if (target === projectId) return;
              move.mutate(
                { projectId: target, resourceIds: [resource.id] },
                {
                  onSuccess: () => {
                    void navigate(`/projects/${target}/resources/${resource.id}/settings`, {
                      replace: true,
                    });
                  },
                },
              );
            }}
          />
        </div>
        <ErrorMessage error={move.error} />
      </Section>

      {resource.kind === 'server' && can(resource.provider, 'server_access') && (
        <ServerAgents resourceId={resource.id} />
      )}

      {watching && (
        <Section title="Remove">
          <div className={styles.settingRow}>
            <div className={styles.settingText}>
              <b>Remove from project</b>
              <span>Hoista stops listing it here. It keeps running on {provider}.</span>
            </div>
            <RemoveFromProject
              resource={resource}
              projectName={project.project.name}
              last={last}
              onRemoved={() => {
                void navigate(`/projects/${projectId}`);
              }}
            />
          </div>
          {can(resource.provider, 'delete') && (
            <div className={styles.settingRow}>
              <div className={styles.settingText}>
                <b>Delete on {provider}</b>
                <span>Deletes it at {provider}. This can’t be undone.</span>
              </div>
              <DeleteAtProvider resource={resource} last={last} />
            </div>
          )}
        </Section>
      )}
    </div>
  );
}

const AGENT_CHOICES = [
  { value: false, label: 'Ask me first' },
  { value: true, label: 'Don’t ask' },
];

/**
 * Whether agents with Full access run commands and write files on this server
 * without asking. Off until the person turns it on; risky steps (firewall,
 * SSH, users, reboots, deleting system folders) ask either way.
 */
function ServerAgents({ resourceId }: { resourceId: string }) {
  const agents = useServerAgents(resourceId);
  const save = useSetServerAgents(resourceId);
  return (
    <Section title="Agents">
      <div className={styles.settingRow}>
        <div className={styles.settingText}>
          <b>Agents with Full access</b>
          <span>
            {save.isPending
              ? 'Saving'
              : 'Whether they ask you before running a command or writing a file here. Changing the firewall, SSH, users or keys, rebooting, and deleting system folders always ask.'}
          </span>
        </div>
        {agents.data === undefined ? (
          <Skeleton width={140} height={32} />
        ) : (
          <Select
            label="Agents with Full access"
            value={agents.data.full_access}
            options={AGENT_CHOICES}
            disabled={save.isPending}
            onChange={(fullAccess) => {
              if (fullAccess !== agents.data.full_access) save.mutate(fullAccess);
            }}
          />
        )}
      </div>
      <ErrorMessage error={agents.error ?? save.error} />
    </Section>
  );
}
