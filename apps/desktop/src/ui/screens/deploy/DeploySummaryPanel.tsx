/** "Going to": where each resource of a deploy is going. */
import { useDeploySummary } from '@ui/api/deploys';
import { Fact, Facts } from '@ui/components/Facts';
import styles from './DeploySummaryPanel.module.css';

export function DeploySummaryPanel({ deployId }: { deployId: string }) {
  const summary = useDeploySummary(deployId);
  if (!summary.data || summary.data.targets.length === 0) return null;

  return (
    <section className={styles.summary} aria-labelledby="going-to-heading">
      <h2 id="going-to-heading" className={styles.label}>
        Going to
      </h2>
      <Facts>
        {summary.data.targets.map((target) => (
          <Fact key={target.label} term={target.label}>
            {target.value}
          </Fact>
        ))}
      </Facts>
    </section>
  );
}
