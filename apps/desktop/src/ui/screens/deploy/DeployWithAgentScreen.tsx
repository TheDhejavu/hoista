/**
 * Deploys: every deploy across your projects, newest first, live as agents run
 * them. Hoista has no deploy form: your coding agent does the work through
 * Hoista's MCP server, so the prompt to give it sits in the toolbar.
 */
import { changeTitle } from '../project/deploys/describe';
import { MoreLines } from '../project/deploys/MoreLines';
import { Rocket, Search } from 'lucide-react';
import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import type { Deploy, DeployStatus } from '@shared/api';
import { useAgents } from '@ui/api/agents';
import { useProjects } from '@ui/api/projects';
import { useRecentDeploys } from '@ui/api/deploys';
import { ActorIcon, AgentIcon } from '@ui/components/AgentIcon';
import { ProjectPreviewIcon } from '@ui/components/ProjectPreviewIcon';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { serviceName } from '@shared/providers';
import { Button } from '@ui/components/Button';
import { EmptyState } from '@ui/components/EmptyState';
import { Page } from '@ui/components/Page';
import { QueryState } from '@ui/components/QueryState';
import { SkeletonTableBody } from '@ui/components/Skeleton';
import { PillSelect, type Option } from '@ui/components/Select';
import { cx } from '@ui/lib/cx';
import { formatDuration, shortAgo } from '@ui/lib/format';
import { useCopy } from '@ui/lib/useCopy';
import { useInEnvironment } from '@ui/lib/useInEnvironment';
import { useCurrentEnvironment } from '@ui/api/environments';
import styles from './DeployWithAgentScreen.module.css';

const FIRST_PROMPT = 'Deploy this project with Hoista';

/**
 * What to ask an agent, from what the deploys show: watch one that's
 * running, look into one that failed, or (with nothing to fix) review the
 * latest. Only with no deploys yet is it "deploy this project".
 */
export function promptFor(deploys: Deploy[], projectName: (id: string) => string): string {
  const running = deploys.find(
    (deploy) => deploy.status === 'running' || deploy.status === 'queued',
  );
  if (running) return `Watch the ${projectName(running.project_id)} deploy until it’s live`;
  // A project whose newest deploy failed comes first: that's what needs fixing.
  const newest = new Map<string, Deploy>();
  for (const deploy of deploys) {
    if (deploy.status !== 'cancelled' && !newest.has(deploy.project_id))
      newest.set(deploy.project_id, deploy);
  }
  const failed = [...newest.values()].find((deploy) => deploy.status === 'failed');
  if (failed) return `Why did ${projectName(failed.project_id)}’s last deploy fail?`;
  const latest = deploys.find((deploy) => deploy.status !== 'cancelled');
  if (!latest) return FIRST_PROMPT;
  return `What changed in ${projectName(latest.project_id)}’s last deploy?`;
}

type StatusFilter = 'live' | 'running' | 'failed' | 'cancelled' | null;

const STATUS_CHOICES: Option<StatusFilter>[] = [
  { value: null, label: 'All' },
  { value: 'running', label: 'Deploying' },
  { value: 'live', label: 'Live' },
  { value: 'failed', label: 'Failed' },
  { value: 'cancelled', label: 'Stopped' },
];

const LOOKS: Record<DeployStatus, { label: string; tone: string }> = {
  queued: { label: 'Waiting', tone: 'warn' },
  running: { label: 'Deploying', tone: 'warn' },
  live: { label: 'Live', tone: 'good' },
  failed: { label: 'Failed', tone: 'bad' },
  cancelled: { label: 'Stopped', tone: 'off' },
  rolled_back: { label: 'Rolled back', tone: 'off' },
};

export function DeployWithAgentScreen() {
  const deploys = useRecentDeploys();
  const inEnvironment = useInEnvironment();
  const environment = useCurrentEnvironment();
  const [search, setSearch] = useState('');
  const [status, setStatus] = useState<StatusFilter>(null);
  const [by, setBy] = useState<string | null>(null);

  // Deploys follow the environment the window shows, like the Projects list.
  const all = (deploys.data?.items ?? []).filter((deploy) => inEnvironment(deploy.project_id));
  const people = [...new Set(all.map(whoOf))];
  const text = search.trim().toLowerCase();
  const shown = all.filter(
    (deploy) =>
      (status === null ||
        deploy.status === status ||
        (status === 'running' && deploy.status === 'queued')) &&
      (by === null || whoOf(deploy) === by) &&
      (!text ||
        [deploy.project_id, deploy.source.message ?? '', deploy.source.ref].some((field) =>
          field.toLowerCase().includes(text),
        )),
  );

  return (
    <Page>
      <h1 className="visually-hidden">Deploys</h1>
      <div className={styles.toolbar}>
        <label className={styles.search}>
          <Search size={14} aria-hidden="true" />
          <input
            type="search"
            aria-label="Filter deploys"
            placeholder="Filter by project, change or commit"
            value={search}
            onChange={(event) => {
              setSearch(event.target.value);
            }}
          />
        </label>
        <PillSelect label="Status" value={status} options={STATUS_CHOICES} onChange={setStatus} />
        <PillSelect
          label="By"
          value={by}
          options={[
            { value: null, label: 'Anyone' },
            ...people.map((name) => ({ value: name, label: name })),
          ]}
          onChange={setBy}
        />
        <AskBar deploys={all} />
      </div>

      <QueryState
        query={deploys}
        skeleton={
          <div className={styles.frame}>
            <table className={styles.table}>
              <DeploysHead />
              <SkeletonTableBody count={6} widths={[24, 120, 200, 90, 80, 30, 50, 50, null]} />
            </table>
          </div>
        }
      >
        {() =>
          all.length === 0 ? (
            <EmptyState icon={<Rocket />} title="No deploys yet">
              Deploys to {environment.environment?.name ?? 'this environment'} from your agent, a{' '}
              <code>git push</code> or <code>hoist deploy</code> show up here.
            </EmptyState>
          ) : (
            <div className={styles.frame}>
              {shown.length === 0 ? (
                <p className={styles.empty}>No deploys match these filters.</p>
              ) : (
                <DeploysTable deploys={shown} />
              )}
            </div>
          )
        }
      </QueryState>
    </Page>
  );
}

/** "You" rather than "You in Hoista Desktop". */
function whoOf(deploy: Deploy): string {
  return deploy.started_by.kind === 'person' ? 'You' : deploy.started_by.name;
}

/** The prompt to give a coding agent, with the tools that are connected. */
function AskBar({ deploys }: { deploys: Deploy[] }) {
  const agents = useAgents();
  const projects = useProjects().data?.items ?? [];
  const prompt = promptFor(
    deploys,
    (id) => projects.find((project) => project.id === id)?.name ?? 'this project',
  );
  const { copied, copy } = useCopy();
  const tools = (agents.data?.items ?? []).filter((agent) => agent.kind === 'coding_tool');

  return (
    <div className={styles.ask}>
      <span className={styles.muted}>Ask a coding agent:</span>
      <b className={styles.prompt} title={prompt}>
        “{prompt}”
      </b>
      <span className={styles.tools}>
        {tools.map((tool) => (
          <AgentIcon key={tool.id} name={tool.name} small />
        ))}
      </span>
      <Button
        onClick={() => {
          copy(prompt);
        }}
      >
        {copied ? 'Copied' : 'Copy'}
      </Button>
    </div>
  );
}

/** The deploys table's header, the same while it loads. */
function DeploysHead() {
  return (
    <thead>
      <tr>
        <th scope="col" colSpan={2}>
          Project
        </th>
        <th scope="col">Change</th>
        <th scope="col">Provider</th>
        <th scope="col">By</th>
        <th scope="col">Version</th>
        <th scope="col">Duration</th>
        <th scope="col" className={styles.right}>
          When
        </th>
        <th scope="col">
          <span className="visually-hidden">Result</span>
        </th>
      </tr>
    </thead>
  );
}

function DeploysTable({ deploys }: { deploys: Deploy[] }) {
  const navigate = useNavigate();
  const projects = useProjects().data?.items ?? [];

  return (
    <table className={styles.table}>
      <DeploysHead />
      <tbody>
        {deploys.map((deploy) => {
          // A version that served before the one live now reads as earlier, as on
          // a project's Deploys tab, not as a second "Live".
          const look =
            deploy.status === 'live' && !deploy.current
              ? { label: 'Earlier', tone: 'off' }
              : LOOKS[deploy.status];
          const project = projects.find((candidate) => candidate.id === deploy.project_id);
          return (
            <tr
              key={deploy.id}
              onClick={() => {
                void navigate(`/deploys/${deploy.id}`);
              }}
            >
              <td className={styles.iconCol}>
                <ProjectPreviewIcon
                  project={{ id: deploy.project_id, url: project?.url ?? '' }}
                  size={22}
                />
              </td>
              <td className={styles.project}>{project?.name ?? deploy.project_id}</td>
              <td className={styles.change}>
                {/* The subject only: the whole message is on the deploy's page. */}
                <span className={styles.subjectRow}>
                  <span className={styles.subject}>{changeTitle(deploy)}</span>
                  <MoreLines message={deploy.source.message} />
                </span>
                <span className={styles.ref}>{deploy.source.ref}</span>
              </td>
              <td>
                <span className={styles.providers}>
                  {(deploy.provider ? [deploy.provider] : (project?.providers ?? []))
                    .slice(0, 3)
                    .map((provider) => (
                      <span key={provider} title={serviceName(provider)}>
                        <ServiceIcon id={provider} size={14} />
                      </span>
                    ))}
                </span>
              </td>
              <td>
                <span className={styles.who}>
                  <ActorIcon actor={deploy.started_by} />
                  {whoOf(deploy)}
                </span>
              </td>
              <td className={cx('mono', styles.nowrap)}>{deploy.version}</td>
              <td className={cx('mono', styles.muted, styles.nowrap)}>
                {deploy.duration_ms === null ? '—' : formatDuration(deploy.duration_ms)}
              </td>
              <td className={cx(styles.muted, styles.right)}>
                {deploy.status === 'running'
                  ? 'now'
                  : shortAgo(deploy.created_at).replace(' ago', '')}
              </td>
              <td>
                <span className={styles.pill}>
                  <span className={cx(styles.dot, styles[look.tone])} />
                  {look.label}
                </span>
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
