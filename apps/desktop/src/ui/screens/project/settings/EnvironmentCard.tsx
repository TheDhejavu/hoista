/**
 * Which Hoista environment the project lives in, and moving it to another.
 * Agents limited to some environments see it only there. Nothing changes at
 * the provider.
 */
import { useState } from 'react';
import type { ProjectDetail } from '@shared/api';
import { useEnvironments, useMoveProject } from '@ui/api/environments';
import { Button } from '@ui/components/Button';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { PillSelect } from '@ui/components/Select';
import { showToast } from '@ui/lib/toast';
import { Card } from './SettingsTab';

export function EnvironmentCard({ detail }: { detail: ProjectDetail }) {
  const { project } = detail;
  const environments = useEnvironments().data ?? [];
  const move = useMoveProject();
  const [target, setTarget] = useState<string | null>(null);
  const here = environments.find((environment) => environment.id === project.environment_id);
  const chosen = environments.find((environment) => environment.id === target);
  if (environments.length < 2) return null;

  return (
    <Card
      title="Environment"
      text={`${project.name} is in ${here?.name ?? 'default'}. Moving it changes where it shows in Hoista and which agents can reach it. Nothing changes at the provider.`}
      footer={
        <>
          <span>Agents limited to {here?.name ?? 'default'} lose access to it.</span>
          <Button
            variant="accent"
            large
            busy={move.isPending}
            busyLabel="Moving"
            disabled={!chosen}
            onClick={() => {
              if (!chosen) return;
              move.mutate(
                { projectId: project.id, environmentId: chosen.id },
                {
                  onSuccess: () => {
                    showToast(`Moved ${project.name} to ${chosen.name}`);
                    setTarget(null);
                  },
                },
              );
            }}
          >
            {chosen ? `Move to ${chosen.name}` : 'Move'}
          </Button>
        </>
      }
    >
      <PillSelect
        label="Move to"
        value={target}
        options={[
          { value: null, label: 'None' },
          ...environments
            .filter((environment) => environment.id !== project.environment_id)
            .map((environment) => ({ value: environment.id, label: environment.name })),
        ]}
        onChange={setTarget}
      />
      <ErrorMessage error={move.error} />
    </Card>
  );
}
