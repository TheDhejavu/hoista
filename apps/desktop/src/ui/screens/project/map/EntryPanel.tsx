/**
 * The side panel for the boxes on the left of the map that aren't
 * resources: where the code comes from (its repository or folder) and the
 * public address. Same card as a resource's, with what can be done from here.
 */
import { Folder, Globe } from 'lucide-react';
import type { ProjectDetail } from '@shared/api';
import { serviceName } from '@shared/providers';
import { openExternal } from '@ui/api/system';
import { Button, ButtonLink } from '@ui/components/Button';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { commitUrl, repoUrl, splitSource } from '@ui/lib/links';
import { formatCount, hostOf, timeAgo } from '@ui/lib/format';
import { runsCode } from '../resourceKinds';
import styles from './SelectedResource.module.css';

export function EntryPanel({
  which,
  detail,
}: {
  which: 'source' | 'address';
  detail: ProjectDetail;
}) {
  return which === 'source' ? <SourcePanel detail={detail} /> : <AddressPanel detail={detail} />;
}

function Facts({ facts }: { facts: [string, string][] }) {
  return (
    <dl className={styles.facts}>
      {facts.map(([term, value]) => (
        <div key={term} className={styles.fact}>
          <dt>{term}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function SourcePanel({ detail }: { detail: ProjectDetail }) {
  const { project } = detail;
  const settings = `/projects/${project.id}/settings`;
  const deployed = project.last_deploy ? timeAgo(project.last_deploy.at) : 'Not yet';

  if (project.source.kind === 'github') {
    const { repo, commit } = splitSource(project.source.ref);
    const repoLink = repoUrl(repo);
    const commitLink = commitUrl(repo, commit);
    return (
      <aside className={styles.card} aria-label="Where the code comes from">
        <div className={styles.header}>
          <span className={styles.icon}>
            <ServiceIcon id="github" size={16} />
          </span>
          <span className={styles.name}>{repo}</span>
        </div>
        <Facts
          facts={[
            ['Kind', 'GitHub repository'],
            ...(commit ? [['Commit', commit] as [string, string]] : []),
            ['Last deploy', deployed],
          ]}
        />
        <div className={styles.actions}>
          {repoLink && (
            <Button
              variant="accent"
              large
              onClick={() => {
                void openExternal(repoLink);
              }}
            >
              Open on GitHub ↗
            </Button>
          )}
          {commitLink && (
            <Button
              large
              onClick={() => {
                void openExternal(commitLink);
              }}
            >
              Open the commit ↗
            </Button>
          )}
          <ButtonLink to={settings} large>
            Change in Settings
          </ButtonLink>
        </div>
      </aside>
    );
  }

  const folder = project.source.kind === 'folder' && project.source.ref ? project.source.ref : null;
  const git = project.source.git;
  return (
    <aside className={styles.card} aria-label="Where the code comes from">
      <div className={styles.header}>
        <span className={styles.icon}>
          {git ? <ServiceIcon id="github" size={16} /> : <Folder size={16} aria-hidden="true" />}
        </span>
        <span className={styles.name}>{git?.repository ?? folder ?? 'Code not linked'}</span>
      </div>
      <Facts
        facts={
          folder
            ? [
                ['Kind', git ? 'Local clone of a GitHub repository' : 'Folder on this computer'],
                ...(git?.branch ? [['Branch', git.branch] as [string, string]] : []),
                ...(git?.commit ? [['Commit', git.commit.slice(0, 7)] as [string, string]] : []),
                [
                  'Folder',
                  git?.temporary ? `${folder} (temporary: the system can clear it)` : folder,
                ],
                ['Last deploy', deployed],
              ]
            : [['Kind', 'Not linked yet']]
        }
      />
      <div className={styles.actions}>
        <ButtonLink to={settings} variant="accent" large>
          {folder ? 'Change in Settings' : 'Link its code in Settings'}
        </ButtonLink>
      </div>
    </aside>
  );
}

function AddressPanel({ detail }: { detail: ProjectDetail }) {
  const { project, stats } = detail;
  const served = detail.resources.find((resource) => runsCode(resource.kind));
  const url = project.url;
  return (
    <aside className={styles.card} aria-label="Public address">
      <div className={styles.header}>
        <span className={styles.icon}>
          {served ? (
            <ServiceIcon id={served.provider} size={16} />
          ) : (
            <Globe size={16} aria-hidden="true" />
          )}
        </span>
        <span className={styles.name}>{url ? hostOf(url) : 'No domain'}</span>
      </div>
      <Facts
        facts={[
          ['Kind', 'Public address'],
          ...(served
            ? [['Goes to', `${served.name} on ${serviceName(served.provider)}`] as [string, string]]
            : []),
          ...(stats.requests !== null
            ? [['Requests', `${formatCount(stats.requests)} in 7 days`] as [string, string]]
            : []),
        ]}
      />
      <div className={styles.actions}>
        {url && (
          <Button
            variant="accent"
            large
            onClick={() => {
              void openExternal(url);
            }}
          >
            Open site ↗
          </Button>
        )}
        <ButtonLink to={`/projects/${project.id}/domains`} large>
          Domains
        </ButtonLink>
      </div>
    </aside>
  );
}
