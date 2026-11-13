/**
 * A deploy in progress: each step with its state, the build output as it
 * arrives, and where everything is going.
 */
import type { Deploy } from '@shared/api';
import { useProject } from '@ui/api/projects';
import { useCancelDeploy } from '@ui/api/deploys';
import { Button } from '@ui/components/Button';
import { timeAgo } from '@ui/lib/format';
import { DeployHead, DeployLog, DeployPill, StepChips } from './DeployResources';
import { DeploySummaryPanel } from './DeploySummaryPanel';
import styles from './DeployingView.module.css';

export function DeployingView({ deploy }: { deploy: Deploy }) {
  const project = useProject(deploy.project_id);
  const name = project.data?.project.name ?? deploy.project_id;
  const cancel = useCancelDeploy();

  return (
    <div className={styles.page}>
      <DeployHead
        deploy={deploy}
        title={`Deploying ${name}`}
        mascot="deploying"
        pill={<DeployPill tone="warn">Deploying</DeployPill>}
        sub={`Started ${timeAgo(deploy.created_at)} by ${deploy.started_by.kind === 'person' ? 'you' : deploy.started_by.name} · you can close this window, Hoista keeps going`}
        actions={
          <Button
            large
            disabled={cancel.isPending}
            title="Stops after the current step. Nothing is released."
            onClick={() => {
              cancel.mutate(deploy.id);
            }}
          >
            {cancel.isPending ? 'Stopping' : 'Stop this deploy'}
          </Button>
        }
      />
      <StepChips steps={deploy.steps} />
      <div className={styles.layout}>
        <DeployLog deployId={deploy.id} height={300} live />
        <aside className={styles.aside}>
          <DeploySummaryPanel deployId={deploy.id} />
        </aside>
      </div>
    </div>
  );
}
