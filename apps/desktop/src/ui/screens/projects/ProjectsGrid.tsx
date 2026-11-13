/**
 * The Projects grid: a drawing of each project's page, its name and address, the
 * providers it runs on, and its status.
 */
import { Link } from 'react-router-dom';
import type { ProjectListItem } from '@shared/api';
import { serviceName } from '@shared/providers';
import { ProjectShot } from '@ui/components/ProjectShot';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { statusLook } from '@ui/lib/status';
import { cx } from '@ui/lib/cx';
import { hostOf } from '@ui/lib/format';
import styles from './ProjectsGrid.module.css';

export function ProjectsGrid({ projects }: { projects: ProjectListItem[] }) {
  return (
    <div className={styles.grid}>
      {projects.map((project) => (
        <ProjectCard key={project.id} project={project} />
      ))}
    </div>
  );
}

function ProjectCard({ project }: { project: ProjectListItem }) {
  const { label, tone } = statusLook(project.status, project.idle_days);
  const failing = project.status === 'failing';

  return (
    <Link to={`/projects/${project.id}`} className={styles.card}>
      <ProjectShot
        project={project}
        height={186}
        badge={
          failing && (
            <span className={styles.badge}>
              <span className={cx(styles.dot, styles.bad)} />
              Failing
            </span>
          )
        }
      />
      <div className={styles.head}>
        <div className={styles.names}>
          <div className={styles.name}>{project.name}</div>
          <div className={styles.address}>{project.url ? hostOf(project.url) : 'No domain'}</div>
        </div>
        <div className={styles.providers}>
          {project.providers.map((provider) => (
            <span key={provider} className={styles.providerTile} title={serviceName(provider)}>
              <ServiceIcon id={provider} size={12} />
            </span>
          ))}
        </div>
      </div>
      <div className={styles.meta}>
        <span className={cx(styles.dot, styles[tone])} />
        <span>{label}</span>
      </div>
    </Link>
  );
}
