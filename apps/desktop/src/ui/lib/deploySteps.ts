/** Plain-English names for the deploy pipeline's steps, used wherever steps are shown. */
import type { StepName } from '@shared/api';

const STEP_LABELS: Record<StepName, string> = {
  detect: 'Read project',
  plan: 'Plan',
  secrets: 'Secrets',
  leak_check: 'Secret scan',
  build: 'Build',
  release: 'Go live',
  verify: 'Health check',
};

export function stepLabel(step: StepName): string {
  return STEP_LABELS[step];
}
