/**
 * Shown on the overview when a project is paused: what that means for visitors,
 * and where to resume it.
 */
import type { ProjectDetail } from '@shared/api';
import { serviceName } from '@shared/providers';
import { listOf } from '@ui/lib/format';
import styles from './PausedNotice.module.css';

export function PausedNotice({ detail }: { detail: ProjectDetail }) {
  const { project, resources } = detail;
  const providers = listOf([
    ...new Set(resources.map((resource) => serviceName(resource.provider))),
  ]);

  return (
    <section className={styles.notice} aria-labelledby="paused-heading">
      <div>
        <h2 id="paused-heading" className={styles.title}>
          {project.name} is paused
        </h2>
        <p className={styles.text}>
          It doesn’t serve requests until it’s resumed. Nothing was deleted, and your data is safe.
          Resume it {providers ? `on ${providers}` : 'at its provider'}, or ask your agent to.
        </p>
      </div>
    </section>
  );
}
