/**
 * Project overview: headline cards (requests, errors, the provider's bill, last deploy),
 * then the project's resources with a question for your agent, and what
 * happened lately.
 */
import { useCan } from '@ui/api/providers';
import { providerEntry, serviceName } from '@shared/providers';
import { ResourceSelect } from '@ui/components/ResourceSelect';
import { useStoredChoice } from '@ui/lib/storedChoice';
import { Link, useNavigate } from 'react-router-dom';
import type { ActivityItem, ProjectDetail, Deploy, Resource, Status } from '@shared/api';
import { useActivity, useProject } from '@ui/api/projects';
import { useDeploys } from '@ui/api/deploys';
import { ActorIcon, AgentIcon } from '@ui/components/AgentIcon';
import { Button } from '@ui/components/Button';
import { QueryState } from '@ui/components/QueryState';
import { Skeleton, SkeletonRows } from '@ui/components/Skeleton';
import { RowMenu } from '@ui/components/RowMenu';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { statusLook } from '@ui/lib/status';
import { cx } from '@ui/lib/cx';
import { formatCount, formatDuration, shortAgo, timeAgo } from '@ui/lib/format';
import { useCopy } from '@ui/lib/useCopy';
import { useRequiredParam } from '@ui/lib/useRequiredParam';
import { resourceKindLabel, resourceTerm, runsCode } from '../resourceKinds';
import { FailingOverview, lastDeployFailed } from './FailingOverview';
import { Bars, Card, ProviderStats, lastDays } from './ProviderStats';
import { PausedNotice } from './PausedNotice';
import { AddResources } from './AddResources';
import styles from './ProjectOverview.module.css';

export function ProjectOverview() {
  const projectId = useRequiredParam('projectId');
  const project = useProject(projectId);
  const deploys = useDeploys(projectId);

  if (project.data && lastDeployFailed(deploys.data?.items ?? [])) {
    return (
      <FailingOverview detail={project.data}>
        <section aria-labelledby="resources-heading">
          <h2 id="resources-heading">Resources</h2>
          <Resources projectId={projectId} resources={project.data.resources} />
        </section>
      </FailingOverview>
    );
  }
  return (
    <div className={styles.page}>
      <QueryState query={project} skeleton={<OverviewSkeleton />}>
        {(detail) => (
          <>
            {detail.project.status === 'paused' && <PausedNotice detail={detail} />}
            <StatCards detail={detail} />
            <div className={styles.columns}>
              <section aria-labelledby="resources-heading">
                <div className={styles.sectionHead}>
                  <h2 id="resources-heading">Resources</h2>
                  <AddResources projectId={projectId} />
                </div>
                <Resources projectId={projectId} resources={detail.resources} />
                {detail.stats.agent_question && <AskBar question={detail.stats.agent_question} />}
              </section>
              <section aria-labelledby="activity-heading">
                <div className={styles.sectionHead}>
                  <h2 id="activity-heading">Activity</h2>
                </div>
                <Activity projectId={projectId} />
              </section>
            </div>
          </>
        )}
      </QueryState>
    </div>
  );
}

/** The overview's shape while it loads: the number cards, then Resources and Activity. */
function OverviewSkeleton() {
  return (
    <div className={styles.skeleton}>
      <div className={styles.stats}>
        {[0, 1, 2, 3].map((card) => (
          <Skeleton key={card} height={96} className={styles.cardSkeleton} />
        ))}
      </div>
      <div className={styles.columns}>
        {['Resources', 'Activity'].map((title) => (
          <section key={title}>
            <div className={styles.sectionHead}>
              <h2>{title}</h2>
            </div>
            <SkeletonRows count={3} columns={2} />
          </section>
        ))}
      </div>
    </div>
  );
}

function StatCards({ detail }: { detail: ProjectDetail }) {
  const { stats } = detail;
  const can = useCan();
  const deploys = useDeploys(detail.project.id);
  // Providers whose own request, response-time and resource numbers the cards read.
  const reports = (resource: Resource): boolean =>
    can(resource.provider, 'metrics') && providerEntry(resource.provider)?.requests !== undefined;
  const services = detail.resources.filter(
    (resource) => reports(resource) && runsCode(resource.kind),
  );
  // The service behind the project's address first, then any that runs code;
  // with several, you pick, and the choice is kept for this project.
  const fallback =
    services.find(
      (resource) =>
        resource.url !== undefined && resource.url !== null && resource.url === detail.project.url,
    ) ?? services[0];
  const [chosen, setChosen] = useStoredChoice<string | null>(
    `hoista.overview.service.${detail.project.id}`,
    null,
    (value): value is string | null => value === null || typeof value === 'string',
  );
  const code = services.find((resource) => resource.id === chosen) ?? fallback;
  // The newest deploy of the service the cards are about; the project's otherwise.
  const items = deploys.data?.items ?? [];
  const mine = code ? items.filter((deploy) => deploy.resource_id === code.id) : [];
  const pool = mine.length > 0 ? mine : items;
  const last = pool.find((deploy) => deploy.current) ?? pool[0];
  // A project of only data (a database, a bucket) has nothing to deploy.
  const ships = detail.project.mode === 'managed' || detail.resources.some((r) => runsCode(r.kind));
  const lastCard = (
    <Card
      label={code && mine.length > 0 ? `Last deploy · ${code.name}` : 'Last deploy'}
      value={last ? shortAgo(last.created_at) : ships ? 'Never' : 'Nothing to deploy'}
      loading={deploys.isLoading}
    >
      {last && <LastDeploy deploy={last} />}
    </Card>
  );
  if (code) {
    const database = detail.resources.find(
      (resource) => reports(resource) && resource.kind === 'database',
    );
    // A group across providers: the other providers' own numbers stay on the overview too.
    const others = [
      ...new Set(
        detail.resources
          .filter((resource) => !reports(resource))
          .map((resource) => resource.provider),
      ),
    ];
    return (
      <>
        {services.length > 1 && (
          <div className={styles.serviceRow}>
            <ResourceSelect
              resources={services}
              value={code.id}
              onChange={(id: string) => {
                setChosen(id);
              }}
            />
            <span className={styles.serviceNote}>
              The numbers below are {code.name}’s, as {serviceName(code.provider)} reports them.
            </span>
          </div>
        )}
        <ProviderStats code={code} database={database ?? null} lastCard={lastCard} />
        {others.length > 0 && (stats.requests !== null || stats.errors_24h !== null) && (
          <div className={cx(styles.stats, styles.statsRow)}>
            {stats.requests !== null && (
              <Card
                label={`Requests · 7 days · ${others.map(serviceName).join(', ')}`}
                value={formatCount(stats.requests)}
              >
                {stats.requests_daily && (
                  <Bars
                    values={stats.requests_daily}
                    labels={lastDays(stats.requests_daily.length)}
                    format={(value) => `${formatCount(value)} requests`}
                    from={lastDays(stats.requests_daily.length)[0]}
                    to="Today"
                  />
                )}
              </Card>
            )}
            {stats.errors_24h !== null && (
              <Card
                label={`Server errors · today · ${others.map(serviceName).join(', ')}`}
                value={formatCount(stats.errors_24h)}
              />
            )}
          </div>
        )}
      </>
    );
  }

  return (
    <div className={styles.stats}>
      {/* Providers that don't count requests (Railway) get no request or error card, not a dash. */}
      {stats.requests !== null && (
        <Card label="Requests · 7 days" value={formatCount(stats.requests)}>
          {stats.requests_daily && (
            <Bars
              values={stats.requests_daily}
              labels={lastDays(stats.requests_daily.length)}
              format={(value) => `${formatCount(value)} requests`}
              from={lastDays(stats.requests_daily.length)[0]}
              to="Today"
            />
          )}
        </Card>
      )}
      {stats.errors_24h !== null && (
        <Card label="Server errors · today" value={formatCount(stats.errors_24h)}>
          {stats.top_error && (
            <div className={cx('mono', styles.note)}>
              {stats.top_error.status} · {stats.top_error.method} {stats.top_error.path}
            </div>
          )}
        </Card>
      )}
      {lastCard}
    </div>
  );
}

function LastDeploy({ deploy }: { deploy: Deploy }) {
  return (
    <div className={styles.deploy}>
      <ActorIcon actor={deploy.started_by} />
      <span className="mono">{deploy.version}</span>
      {deploy.duration_ms !== null && (
        <span className={styles.muted}>· built in {formatDuration(deploy.duration_ms)}</span>
      )}
    </div>
  );
}

function Resources({ projectId, resources }: { projectId: string; resources: Resource[] }) {
  const navigate = useNavigate();

  return (
    <div className={styles.table}>
      {resources.map((resource) => {
        const page = `/projects/${projectId}/resources/${resource.id}`;
        const name = resource.name || resourceKindLabel(resource.kind);
        return (
          <div key={resource.id} className={styles.resourceRow}>
            <button
              type="button"
              className={cx(styles.row, styles.rowInner)}
              onClick={() => {
                void navigate(page);
              }}
            >
              <span className={styles.logo} title={serviceName(resource.provider)}>
                <ServiceIcon id={resource.provider} size={14} />
              </span>
              <span className={styles.named}>
                <span className={styles.resourceName}>{name}</span>
                <span className={styles.product}>{resourceTerm(resource)}</span>
              </span>
              <span className={cx('mono', styles.usage)}>{shortUsage(resource.usage)}</span>
              <StatusPill status={resource.status} />
            </button>
            <RowMenu
              label={`More for ${name}`}
              items={[
                {
                  label: 'Move to another project',
                  onSelect: () => {
                    void navigate(`${page}/settings`);
                  },
                },
              ]}
            />
          </div>
        );
      })}
    </div>
  );
}

/** The first figure of the usage line, such as `212 MB` from `212 MB of 512 MB`. */
function shortUsage(usage: string | null): string {
  if (!usage) return '';
  const figure = /^[\d.,]+\s?[a-zA-Z]+/.exec(usage);
  return figure ? figure[0] : '';
}

function StatusPill({ status }: { status: Status }) {
  const { label, tone } = statusLook(status);
  return (
    <span className={cx(styles.pill, status === 'failing' && styles.pillBad)}>
      <span className={cx(styles.dot, styles[tone])} />
      {label}
    </span>
  );
}

function AskBar({ question }: { question: string }) {
  const { copied, copy } = useCopy();
  return (
    <div className={styles.ask}>
      <span>
        Ask your agent: <b>“{question}”</b>
      </span>
      <Button
        large
        onClick={() => {
          copy(question);
        }}
      >
        {copied ? 'Copied' : 'Copy'}
      </Button>
    </div>
  );
}

/** How many things the overview lists; the rest are on the Activity page. */
const RECENT = 6;

function Activity({ projectId }: { projectId: string }) {
  const activity = useActivity(projectId);
  const items = (activity.data?.items ?? []).filter((item) => !item.housekeeping);

  if (activity.isSuccess && items.length === 0) {
    return <div className={styles.empty}>Nothing has happened yet.</div>;
  }
  return (
    <>
      <ol className={styles.table}>
        {items.slice(0, RECENT).map((item) => (
          <ActivityRow key={item.id} projectId={projectId} item={item} />
        ))}
      </ol>
      {items.length > 0 && (
        <Link to={`/projects/${projectId}/activity`} className={styles.allActivity}>
          See all activity
        </Link>
      )}
    </>
  );
}

/**
 * One activity line. On the Activity tab it selects the line to show its
 * record beside the list; elsewhere it opens the Activity tab on that line.
 */
export function ActivityRow({
  projectId,
  item,
  selected = false,
  onSelect,
}: {
  projectId: string;
  item: ActivityItem;
  selected?: boolean;
  onSelect?: () => void;
}) {
  const body = (
    <>
      {item.actor.kind === 'agent' ? (
        <AgentIcon name={item.actor.name} small />
      ) : (
        <ActorIcon actor={item.actor} />
      )}
      <span className={styles.what}>
        {item.title}
        <span className={styles.muted}> · {item.actor.name}</span>
      </span>
      <span className={styles.muted}>{timeAgo(item.at)}</span>
    </>
  );
  return (
    <li>
      {onSelect ? (
        <button
          type="button"
          className={cx(styles.row, selected && styles.rowSelected)}
          aria-pressed={selected}
          onClick={onSelect}
        >
          {body}
        </button>
      ) : (
        <Link
          to={`/projects/${projectId}/activity?item=${encodeURIComponent(item.id)}`}
          className={styles.row}
        >
          {body}
        </Link>
      )}
    </li>
  );
}
