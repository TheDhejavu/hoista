/**
 * The Projects screen while getting started: the projects added so far as cards,
 * what else is on the connected providers ready to add, and the Get started
 * checklist beside them. Once the checklist is hidden, the full table with
 * its filters takes over.
 */
import { Link } from 'react-router-dom';
import type { Account, ProjectListItem, ProviderId } from '@shared/api';
import { serviceName } from '@shared/providers';
import { useAccounts } from '@ui/api/accounts';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { plural } from '@ui/lib/format';
import { ProjectsGrid } from './ProjectsGrid';
import { FoundOnProviders } from './FoundOnProviders';
import { GetStarted } from './GetStarted';
import styles from './FirstProjects.module.css';

export function FirstProjects({
  projects,
  onHide,
  onConnect,
}: {
  projects: ProjectListItem[];
  onHide: () => void;
  onConnect?: (provider: ProviderId) => void;
}) {
  const providers = (useAccounts().data?.items ?? []).filter(
    (account) => account.service !== 'github',
  );
  // Only the first provider gets the hello; after that the cards speak for themselves.
  const first = providers.length === 1 ? providers[0] : undefined;

  return (
    <div className={styles.screen}>
      <div className={styles.main}>
        {first && <FoundBanner account={first} count={projects.length} />}
        <h1 className={styles.title}>
          Projects <span className={styles.count}>{projects.length}</span>
        </h1>
        <ProjectsGrid projects={projects} />
        <div className={styles.found}>
          <FoundOnProviders onConnect={onConnect} />
        </div>
      </div>
      <GetStarted onHide={onHide} />
    </div>
  );
}

/** "Railway is connected · found 3 projects", and that nothing changed there. */
function FoundBanner({ account, count }: { account: Account; count: number }) {
  const name = serviceName(account.service);

  return (
    <div className={styles.banner} role="status">
      <span className={styles.logo}>
        <ServiceIcon id={account.service} size={18} />
      </span>
      <div className={styles.text}>
        <strong>
          {name} is connected · found {plural(count, 'project')}
        </strong>
        <span>
          {account.mode === 'watching'
            ? `Hoista reads these and changes nothing on ${name}. Add the ones you want to watch.`
            : `Hoista can manage these now. Nothing has changed on ${name}.`}
        </span>
      </div>
      <Link to="/accounts" className={styles.another}>
        Connect another provider
      </Link>
    </div>
  );
}
