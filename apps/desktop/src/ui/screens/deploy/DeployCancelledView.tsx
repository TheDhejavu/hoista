/**
 * A deploy someone stopped before it went live. Nothing on the live site
 * changed, so the page says where it stopped, who stopped it, and how to go
 * again.
 */
import type { Deploy } from '@shared/api';
import { useProject } from '@ui/api/projects';
import { ButtonLink } from '@ui/components/Button';
import { stepLabel } from '@ui/lib/deploySteps';
import { formatDuration } from '@ui/lib/format';
import { AskAgain } from './DeployFailedView';
import { DeployHead, DeployLog, DeployPill, StepChips } from './DeployResources';
import styles from './DeployFailedView.module.css';

export function DeployCancelledView({ deploy }: { deploy: Deploy }) {
  const project = useProject(deploy.project_id);
  const name = project.data?.project.name ?? deploy.project_id;
  // The step it was on when stopped: the first one that never finished.
  const stoppedAt = deploy.steps.find(
    (step) => step.status !== 'done' && step.status !== 'skipped',
  );
  const who = deploy.started_by.kind === 'person' ? 'you' : deploy.started_by.name;

  return (
    <div className={styles.page}>
      <DeployHead
        deploy={deploy}
        title={`${name} deploy stopped`}
        pill={<DeployPill tone="off">Stopped</DeployPill>}
        sub={[
          `Started by ${who}`,
          deploy.duration_ms === null
            ? null
            : `stopped after ${formatDuration(deploy.duration_ms)}`,
          'nothing was released',
        ]
          .filter(Boolean)
          .join(' · ')}
        actions={
          <ButtonLink to={`/projects/${deploy.project_id}/deploys`} large>
            All deploys of {name}
          </ButtonLink>
        }
      />
      <StepChips steps={deploy.steps} stoppedAt={stoppedAt?.name} />
      <div className={styles.columns}>
        <div className={styles.card}>
          <h2>What happened</h2>
          <p className={styles.what}>
            {deploy.failure?.message ??
              `Stopped${stoppedAt ? ` during ${stepLabel(stoppedAt.name).toLowerCase()}` : ''}, before anything was released.`}{' '}
            The previous release is still in production.
          </p>
          <AskAgain
            projectName={name}
            prompt={
              deploy.started_by.kind === 'system'
                ? `Using Hoista, check whether ${name} runs the newest commit${deploy.source.message ? ` (“${deploy.source.message}”)` : ''}. If ${deploy.version} was never released and it's still needed, deploy it.`
                : undefined
            }
          />
        </div>
        <DeployLog deployId={deploy.id} height={200} />
      </div>
    </div>
  );
}
