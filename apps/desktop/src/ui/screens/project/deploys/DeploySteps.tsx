/**
 * The pipeline of one deploy, read left to right: each step with a bar
 * coloured by how it went and how long it took. A line above says the total
 * and where most of the time went.
 */
import type { Deploy, DeployStep } from '@shared/api';
import { stepLabel } from '@ui/lib/deploySteps';
import { formatDuration } from '@ui/lib/format';
import { cx } from '@ui/lib/cx';
import styles from './DeploySteps.module.css';

export function DeploySteps({ deploy }: { deploy: Deploy }) {
  return (
    <div className={styles.pipeline}>
      <Summary steps={deploy.steps} />
      <ol className={styles.steps} aria-label={`Steps of the ${deploy.version} deploy`}>
        {deploy.steps.map((step) => (
          <li
            key={step.name}
            className={cx(styles.step, styles[step.status])}
            title={step.detail ?? undefined}
          >
            <span className={styles.bar} aria-hidden="true" />
            <span className={styles.label}>{stepLabel(step.name)}</span>
            <span className={styles.time}>{stepTime(step)}</span>
          </li>
        ))}
      </ol>
    </div>
  );
}

/** `8 steps · 38s · most of it in Build (16s)`, or which step failed and why. */
function Summary({ steps }: { steps: DeployStep[] }) {
  const failed = steps.find((step) => step.status === 'failed');
  if (failed) {
    return (
      <p className={styles.summary}>
        Stopped at <strong>{stepLabel(failed.name)}</strong>
        {failed.detail && ` · ${failed.detail}`}
      </p>
    );
  }

  const timed = steps.filter((step) => step.duration_ms !== null);
  const total = timed.reduce((sum, step) => sum + (step.duration_ms ?? 0), 0);
  const longest = timed.reduce<DeployStep | null>(
    (slowest, step) => ((step.duration_ms ?? 0) > (slowest?.duration_ms ?? 0) ? step : slowest),
    null,
  );

  return (
    <p className={styles.summary}>
      {steps.length} steps · {formatDuration(total)}
      {longest?.duration_ms != null &&
        ` · most of it in ${stepLabel(longest.name)} (${formatDuration(longest.duration_ms)})`}
    </p>
  );
}

function stepTime(step: DeployStep): string {
  if (step.status === 'skipped') return 'skipped';
  if (step.status === 'running') return 'running';
  return step.duration_ms === null ? '—' : formatDuration(step.duration_ms);
}
