/**
 * The panel under the menu bar icon, laid out like a menu: Hoista and the
 * switch that lets agents reach it; what waits for you (agents asking to
 * connect and reconnects, each with the button that settles it), deploys running now
 * and problems; your projects, each opening to its numbers and the few things
 * people do from a menu; then agents, accounts, and the usual menu items.
 *
 * Approvals aren't here: the agent asks the person in its own window.
 */
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { ChevronRight } from 'lucide-react';
import type { ProjectListItem, AttentionItem, Deploy, WaitingOnAccount } from '@shared/api';
import { reconnectPath } from '@shared/reconnect';
import { SERVICES, serviceName } from '@shared/providers';
import { useAccounts, useBills } from '@ui/api/accounts';
import { useAgents, useConnectionRequests } from '@ui/api/agents';
import { useProjects, useAttention } from '@ui/api/projects';
import { getBridge } from '@ui/api/bridge';
import { useRecentDeploys } from '@ui/api/deploys';
import { useSettings, useUpdateSettings } from '@ui/api/settings';
import { openExternal } from '@ui/api/system';
import { AgentIcon } from '@ui/components/AgentIcon';
import { Dot } from '@ui/components/Dot';
import { LogoMark } from '@ui/components/icons';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { remotePlace } from '@ui/lib/remoteProviders';
import { billSummary } from '@ui/lib/bills';
import { stepLabel } from '@ui/lib/deploySteps';
import { elapsedClock, formatCost, formatCount, plural, shortAgo } from '@ui/lib/format';
import { statusLook } from '@ui/lib/status';
import { useNow } from '@ui/lib/useNow';
import styles from './MenuBarPanel.module.css';

const IS_MAC = getBridge().platform === 'darwin';
const PROJECTS_SHOWN = 5;

function showMain(target?: string): void {
  void getBridge().showMain(target);
}

export function MenuBarPanel() {
  const requests = useConnectionRequests().data?.items ?? [];
  const attention = useAttention();
  const deploys = useRecentDeploys();
  const projects = useProjects();
  const root = useRef<HTMLDivElement>(null);

  const items = attention.data?.items ?? [];
  const reconnects = items.filter((item) => item.waiting_on !== null);
  const problems = items.filter((item) => item.waiting_on === null && item.kind === 'failure');
  const running = (deploys.data?.items ?? []).filter(
    (deploy) => deploy.status === 'running' || deploy.status === 'queued',
  );
  const waiting = reconnects.length + requests.length;
  const agentNames = (useAgents().data?.items ?? []).map((agent) => agent.name);
  const accountNames = [
    ...new Set((useAccounts().data?.items ?? []).map((account) => serviceName(account.service))),
  ];
  const names = new Map((projects.data?.items ?? []).map((project) => [project.id, project.name]));
  const totals = projects.data?.totals;
  const bill = billSummary(useBills().data);

  // The panel window fits its content, so it never scrolls when it needn't.
  useEffect(() => {
    const element = root.current;
    if (!element) return undefined;
    const observer = new ResizeObserver(() => {
      void getBridge().resizePanel(element.scrollHeight);
    });
    observer.observe(element);
    return () => {
      observer.disconnect();
    };
  }, []);

  // Esc hides the panel; the shortcuts match the footer.
  useEffect(() => {
    function onKey(event: KeyboardEvent): void {
      const mod = IS_MAC ? event.metaKey : event.ctrlKey;
      if (event.key === 'Escape') void getBridge().closePanel();
      else if (mod && event.key === 'o') showMain();
      else if (mod && event.key === ',') showMain('settings');
      else if (mod && event.key === 'q') void getBridge().quit();
      else return;
      event.preventDefault();
    }
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('keydown', onKey);
    };
  }, []);

  return (
    <div ref={root} className={styles.panel}>
      <Header waiting={waiting} agents={agentNames.length} />

      {reconnects.length + requests.length > 0 && (
        <Section title="Waiting for you">
          {requests.map((request) => (
            <div key={request.id} className={styles.card}>
              <AgentIcon name={request.claimed_name} />
              <div className={styles.cardText}>
                <p className={styles.cardTitle}>{request.claimed_name} wants to connect</p>
                <p className={styles.cardBody}>
                  {request.via === 'this_computer'
                    ? `On this computer · code ${request.code ?? ''}`
                    : 'Through your Hoista URL'}
                </p>
                <div className={styles.actions}>
                  <button
                    type="button"
                    className={styles.strong}
                    onClick={() => {
                      showMain('/agents');
                    }}
                  >
                    Review
                  </button>
                </div>
              </div>
            </div>
          ))}
          {reconnects.map((item) =>
            item.waiting_on ? (
              <ReconnectCard key={item.id} item={item} waiting={item.waiting_on} />
            ) : null,
          )}
        </Section>
      )}
      {running.length > 0 && (
        <Section title="Deploying now">
          {running.map((deploy) => (
            <DeployCard key={deploy.id} deploy={deploy} name={names.get(deploy.project_id)} />
          ))}
        </Section>
      )}
      {problems.length > 0 && (
        <Section title="Needs a look" rows>
          {problems.map((item) => (
            <ProblemRow key={item.id} item={item} />
          ))}
        </Section>
      )}

      <ProjectList projects={projects.data?.items ?? []} total={totals?.projects ?? 0} />

      <div className={styles.links}>
        <LinkRow
          label="Agents"
          value={agentNames.length > 0 ? listShort(agentNames) : 'None connected'}
          onClick={() => {
            showMain('/agents');
          }}
        />
        <LinkRow
          label="Accounts"
          value={[
            accountNames.length > 0 ? listShort(accountNames) : 'None connected',
            bill ? `${formatCost(bill.total)} used so far` : null,
          ]
            .filter(Boolean)
            .join(' · ')}
          onClick={() => {
            showMain('/accounts');
          }}
        />
      </div>

      <footer className={styles.footer}>
        <MenuItem
          label="Open Hoista"
          keys="O"
          onClick={() => {
            showMain();
          }}
        />
        <MenuItem
          label="Settings"
          keys=","
          onClick={() => {
            showMain('settings');
          }}
        />
        <MenuItem label="Quit Hoista" keys="Q" onClick={() => void getBridge().quit()} />
      </footer>
    </div>
  );
}

function Header({ waiting, agents }: { waiting: number; agents: number }) {
  const settings = useSettings();
  const update = useUpdateSettings();
  const enabled = settings.data?.agents.connection_enabled ?? true;
  const remote = settings.data?.agents.remote;
  const where =
    !remote || remote.target === 'this_computer'
      ? 'On this computer'
      : `Remote · ${remotePlace(remote.target)}`;
  const line = !enabled
    ? 'Agents can’t reach Hoista'
    : waiting > 0
      ? `${String(waiting)} waiting for you`
      : agents > 0
        ? `${where} · ${plural(agents, 'agent')}`
        : where;

  return (
    <header className={styles.header}>
      <span className={styles.projectIcon}>
        <LogoMark size={22} />
      </span>
      <span className={styles.heading}>
        <span className={styles.name}>Hoista</span>
        <span className={styles.where}>{line}</span>
      </span>
      <button
        type="button"
        role="switch"
        aria-checked={enabled}
        aria-label="Let agents reach Hoista"
        title={enabled ? 'Agents can reach Hoista' : 'Agents can’t reach Hoista'}
        className={styles.switch}
        disabled={!settings.data || update.isPending}
        onClick={() => {
          update.mutate({ agents: { connection_enabled: !enabled } });
        }}
      />
    </header>
  );
}

/** Your projects, failing ones first. One opens to its numbers and actions. */
function ProjectList({ projects, total }: { projects: ProjectListItem[]; total: number }) {
  const [openId, setOpenId] = useState<string | null>(null);
  if (projects.length === 0) return null;
  const order = { failing: 0, deploying: 1, paused: 2, unknown: 3 } as Record<string, number>;
  const shown = [...projects]
    .sort(
      (a, b) =>
        (order[a.status] ?? 9) - (order[b.status] ?? 9) ||
        (b.last_deploy?.at ?? '').localeCompare(a.last_deploy?.at ?? ''),
    )
    .slice(0, PROJECTS_SHOWN);

  return (
    <Section title="Projects" rows>
      {shown.map((item) => {
        const look = statusLook(item.status, item.idle_days);
        const opened = openId === item.id;
        return (
          <div key={item.id}>
            <button
              type="button"
              className={styles.row}
              aria-expanded={opened}
              onClick={() => {
                setOpenId(opened ? null : item.id);
              }}
            >
              <Dot tone={look.tone} small />
              <span className={styles.rowText}>
                <span className={styles.rowTitle}>{item.name}</span>
              </span>
              <span className={styles.rowValue}>{look.label}</span>
              <ChevronRight
                size={14}
                className={opened ? styles.chevronOpen : styles.chevron}
                aria-hidden="true"
              />
            </button>
            {opened && <ProjectDetail project={item} />}
          </div>
        );
      })}
      {total > shown.length && (
        <button
          type="button"
          className={styles.row}
          onClick={() => {
            showMain('/');
          }}
        >
          <span className={styles.rowMuted}>All {String(total)} projects</span>
        </button>
      )}
    </Section>
  );
}

function ProjectDetail({ project }: { project: ProjectListItem }) {
  const facts = [
    project.runs_on.length > 0 ? project.runs_on.join(', ') : null,
    project.requests_7d !== null ? `${formatCount(project.requests_7d)} requests · 7 days` : null,
    project.last_deploy ? `Last deploy ${shortAgo(project.last_deploy.at)}` : 'Not deployed yet',
  ].filter(Boolean);
  return (
    <div className={styles.projectDetail}>
      <p className={styles.projectFacts}>{facts.join(' · ')}</p>
      <div className={styles.projectActions}>
        {project.url && (
          <button
            type="button"
            className={styles.quiet}
            onClick={() => {
              void openExternal(project.url ?? '');
            }}
          >
            Open site ↗
          </button>
        )}
        <button
          type="button"
          className={styles.quiet}
          onClick={() => {
            showMain(`/projects/${project.id}/logs`);
          }}
        >
          Logs
        </button>
        <button
          type="button"
          className={styles.strong}
          onClick={() => {
            showMain(`/projects/${project.id}`);
          }}
        >
          Open in Hoista
        </button>
      </div>
    </div>
  );
}

function LinkRow({ label, value, onClick }: { label: string; value: string; onClick: () => void }) {
  return (
    <button type="button" className={styles.row} onClick={onClick}>
      <span className={styles.rowText}>
        <span className={styles.rowTitle}>{label}</span>
      </span>
      <span className={styles.rowValue}>{value}</span>
      <ChevronRight size={14} className={styles.chevron} aria-hidden="true" />
    </button>
  );
}

/** "Claude Code, Codex" or "Claude Code, Codex +2". */
function listShort(names: string[]): string {
  return names.length > 2
    ? `${names.slice(0, 2).join(', ')} +${String(names.length - 2)}`
    : names.join(', ');
}

function Section({
  title,
  rows = false,
  children,
}: {
  title: string;
  rows?: boolean;
  children: ReactNode;
}) {
  return (
    <section className={styles.section} aria-label={title}>
      <h2 className={styles.sectionTitle}>{title}</h2>
      <div className={rows ? styles.rows : styles.cards}>{children}</div>
    </section>
  );
}

function ReconnectCard({ item, waiting }: { item: AttentionItem; waiting: WaitingOnAccount }) {
  const provider = SERVICES[waiting.service].name;
  return (
    <div className={styles.card}>
      <AgentIcon name={waiting.agent} />
      <div className={styles.cardText}>
        <p className={styles.cardTitle}>{item.title}</p>
        <p className={styles.cardBody}>{item.body}</p>
        <div className={styles.actions}>
          <button
            type="button"
            className={styles.strong}
            aria-label={`Reconnect ${provider}`}
            onClick={() => {
              showMain(reconnectPath(waiting));
            }}
          >
            <span className={styles.providerLogo}>
              <ServiceIcon id={waiting.service} size={14} />
            </span>
            Reconnect
          </button>
        </div>
      </div>
    </div>
  );
}

function DeployCard({ deploy, name }: { deploy: Deploy; name: string | undefined }) {
  const total = deploy.steps.length;
  const done = deploy.steps.filter(
    (step) => step.status === 'done' || step.status === 'skipped',
  ).length;
  const current = deploy.steps.find((step) => step.status === 'running');
  const at = Math.min(total, done + 1);
  const now = useNow();
  const elapsed = elapsedClock(now - Date.parse(deploy.created_at)).replace(/^0/, '');
  const agent = deploy.started_by.kind === 'agent' ? deploy.started_by.name : null;

  return (
    <button
      type="button"
      className={styles.card}
      onClick={() => {
        showMain(`/deploys/${deploy.id}`);
      }}
    >
      {agent ? <AgentIcon name={agent} /> : <AgentIcon name="Hoista" person />}
      <div className={styles.cardText}>
        <p className={styles.cardTitle}>
          {name ?? deploy.project_id} {deploy.version}
        </p>
        <p className={styles.cardBody}>
          {deploy.status === 'queued'
            ? 'Waiting to start'
            : `${current ? stepLabel(current.name) : 'In progress'} · step ${String(at)} of ${String(total)} · `}
          {deploy.status !== 'queued' && <span className={styles.clock}>{elapsed}</span>}
        </p>
        <span className={styles.progress} aria-hidden="true">
          <span style={{ width: `${String(total ? (done / total) * 100 : 0)}%` }} />
        </span>
      </div>
    </button>
  );
}

function ProblemRow({ item }: { item: AttentionItem }) {
  const target = item.project_id
    ? item.kind === 'failure'
      ? `/projects/${item.project_id}/env`
      : `/projects/${item.project_id}`
    : '/';
  return (
    <button
      type="button"
      className={styles.row}
      onClick={() => {
        showMain(target);
      }}
    >
      <Dot tone={item.tone} small />
      <span className={styles.rowText}>
        <span className={styles.rowTitle}>{item.title}</span>
        <span className={styles.rowBody}>{item.body}</span>
      </span>
      <ChevronRight size={14} className={styles.chevron} aria-hidden="true" />
    </button>
  );
}

function MenuItem({ label, keys, onClick }: { label: string; keys: string; onClick: () => void }) {
  return (
    <button type="button" className={styles.menuItem} onClick={onClick}>
      <span>{label}</span>
      <kbd className={styles.keys}>{IS_MAC ? `⌘${keys}` : `Ctrl+${keys}`}</kbd>
    </button>
  );
}
