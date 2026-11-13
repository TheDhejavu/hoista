/**
 * Why a provider's numbers aren't showing, the way providers' dashboards say it: a
 * plan that doesn't include them is a quiet note with where to read about
 * the plan; a provider that failed to answer is a faint red one with its words.
 */
import { Info, TriangleAlert } from 'lucide-react';
import { EngineError } from '@shared/engine';
import { providerEntry, serviceName } from '@shared/providers';
import { ExternalLink } from '@ui/components/ExternalLink';
import { cx } from '@ui/lib/cx';
import styles from './MetricsNotice.module.css';

export function MetricsNotice({ provider, error }: { provider: string; error: Error }) {
  const name = serviceName(provider as Parameters<typeof serviceName>[0]);
  const plan = error instanceof EngineError && error.code === 'provider.plan';
  const planUrl = providerEntry(provider)?.charts?.plan;
  return (
    <div className={cx(styles.notice, !plan && styles.bad)} role={plan ? 'note' : 'alert'}>
      {plan ? (
        <Info size={16} aria-hidden="true" className={styles.icon} />
      ) : (
        <TriangleAlert size={16} aria-hidden="true" className={styles.icon} />
      )}
      <div className={styles.text}>
        <strong>{plan ? `Metrics need a ${name} plan` : `${name} didn’t send its numbers`}</strong>
        <span>{error.message}</span>
      </div>
      {plan && planUrl && (
        <ExternalLink variant="text" url={planUrl} className={styles.link}>
          See the plan
        </ExternalLink>
      )}
    </div>
  );
}
