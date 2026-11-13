/**
 * The connected agents: what each may do, the projects it can reach, when it
 * was last active, and a quiet menu for editing its access and disconnecting.
 * Coding tools are limited like any other agent: a new one starts able to
 * view only.
 */
import { useEffect, useRef, useState } from 'react';
import type { Agent } from '@shared/api';
import { Bot, MoreVertical, ShieldCheck } from 'lucide-react';
import { useAgents, useDisconnectAgent, useSetAgentAccess } from '@ui/api/agents';
import { useProjects } from '@ui/api/projects';
import { ALL_ENVIRONMENTS } from '@ui/api/environments';
import { AgentIcon } from '@ui/components/AgentIcon';
import { Button } from '@ui/components/Button';
import { EmptyState } from '@ui/components/EmptyState';
import { QueryState } from '@ui/components/QueryState';
import { SkeletonRows } from '@ui/components/Skeleton';
import { PillSelect, type Option } from '@ui/components/Select';
import { ACCESS_CHOICES, ACCESS_HINTS, ALWAYS_ASKS } from '@ui/lib/agentAccess';
import { cx } from '@ui/lib/cx';
import { plural, timeAgo } from '@ui/lib/format';
import { AgentLimitsDialog } from './AgentLimitsDialog';
import styles from './AgentsTable.module.css';

/** What the second line under an agent's name says. */
function kindLabel(agent: Agent): string {
  if (agent.kind === 'chat_app') return 'Chat app';
  if (agent.kind === 'coding_tool') return 'Coding tool · this computer';
  return agent.webhook ? 'Cloud agent · webhook on' : 'Cloud agent · runs on its own';
}

type KindFilter = Agent['kind'] | null;

const KIND_CHOICES: Option<KindFilter>[] = [
  { value: null, label: 'All' },
  { value: 'chat_app', label: 'Chat apps' },
  { value: 'cloud_agent', label: 'Cloud agents' },
  { value: 'coding_tool', label: 'Coding tools' },
];

/** Waiting first, then agents with their own limits, then tools that run as you; newest first within each. */
function byAttention(a: Agent, b: Agent): number {
  const rank = (agent: Agent): number =>
    agent.status === 'waiting' ? 0 : agent.kind === 'coding_tool' ? 2 : 1;
  const seen = (agent: Agent): number => (agent.last_seen ? Date.parse(agent.last_seen) : 0);
  return rank(a) - rank(b) || seen(b) - seen(a);
}

export function AgentsTable({ onConnect }: { onConnect: () => void }) {
  const agents = useAgents();
  const [kind, setKind] = useState<KindFilter>(null);
  const all = agents.data?.items ?? [];
  const none = agents.isSuccess && all.length === 0;
  const shown = all.filter((agent) => kind === null || agent.kind === kind).sort(byAttention);
  // Per-agent project lists show only where an agent has one.
  const columns: Columns = {
    projects: all.some((agent) => agent.projects !== null),
  };
  const grid = {
    gridTemplateColumns: [
      '30px',
      '200px',
      '220px',
      columns.projects ? 'minmax(0, 1fr)' : null,
      columns.projects ? '110px' : 'minmax(110px, 1fr)',
      '24px',
    ]
      .filter(Boolean)
      .join(' '),
  };

  return (
    <section className={styles.section} aria-labelledby="connected-heading">
      <div className={styles.sectionHead}>
        <h2 id="connected-heading" className={styles.sectionTitle}>
          Connected
        </h2>
        <span className={styles.count}>{plural(all.length, 'agent')}</span>
        <span className={styles.spacer} />
        {!none && (
          <PillSelect label="Kind" value={kind} options={KIND_CHOICES} onChange={setKind} />
        )}
        <Button variant="accent" large onClick={onConnect}>
          Connect an agent
        </Button>
      </div>
      {none ? (
        <EmptyState icon={<Bot />} title="No agents connected">
          Coding tools that run <code>hoist mcp</code> and chat apps you approve show up here, each
          with what it may do.
        </EmptyState>
      ) : (
        <div className={styles.frame}>
          <div className={cx(styles.row, styles.headRow)} style={grid}>
            <span />
            <span>Agent</span>
            <span>What it can do</span>
            {columns.projects && <span>Projects it can reach</span>}
            <span className={styles.right}>Last active</span>
            <span />
          </div>
          <QueryState query={agents} skeleton={<SkeletonRows count={3} columns={4} />}>
            {() =>
              shown.length === 0 ? (
                <p className={styles.empty}>
                  No agents yet. Connect one to manage projects by asking.
                </p>
              ) : (
                shown.map((agent) => (
                  <AgentRow key={agent.id} agent={agent} columns={columns} grid={grid} />
                ))
              )
            }
          </QueryState>
        </div>
      )}
      <p className={styles.explain}>
        <ShieldCheck size={15} aria-hidden="true" />
        {ALWAYS_ASKS}
      </p>
    </section>
  );
}

interface Columns {
  projects: boolean;
}

function AgentRow({
  agent,
  columns,
  grid,
}: {
  agent: Agent;
  columns: Columns;
  grid: { gridTemplateColumns: string };
}) {
  const setAccess = useSetAgentAccess();
  const projectNames = new Map(
    (useProjects({ environment: ALL_ENVIRONMENTS }).data?.items ?? []).map((project) => [
      project.id,
      project.name,
    ]),
  );
  const [isEditingLimits, setEditingLimits] = useState(false);

  return (
    <div className={styles.row} style={grid}>
      <AgentIcon name={agent.name} />
      <div className={styles.stack}>
        <span>{agent.name}</span>
        <span className={styles.hint}>{kindLabel(agent)}</span>
      </div>
      <div>
        {
          <span title={ACCESS_HINTS[agent.access]}>
            <PillSelect
              label={`What ${agent.name} can do`}
              hideLabel
              value={agent.access}
              options={ACCESS_CHOICES}
              onChange={(access) => {
                setAccess.mutate({ agentId: agent.id, access });
              }}
            />
          </span>
        }
      </div>
      {columns.projects && (
        <span className={styles.muted}>
          {agent.projects === null
            ? 'Every project'
            : agent.projects.map((id) => projectNames.get(id) ?? id).join(', ')}
        </span>
      )}
      <LastActive agent={agent} />
      <RowMenu
        agent={agent}
        onEditLimits={() => {
          setEditingLimits(true);
        }}
      />
      {isEditingLimits && (
        <AgentLimitsDialog
          agent={agent}
          onClose={() => {
            setEditingLimits(false);
          }}
        />
      )}
    </div>
  );
}

function LastActive({ agent }: { agent: Agent }) {
  if (agent.status === 'waiting') {
    return <span className={cx(styles.lastSeen, styles.waiting)}>Waiting for you</span>;
  }
  return (
    <span className={styles.lastSeen}>{agent.last_seen ? timeAgo(agent.last_seen) : 'Never'}</span>
  );
}

/** "…" with the rare actions, so every row isn't shouting Disconnect. */
function RowMenu({ agent, onEditLimits }: { agent: Agent; onEditLimits?: () => void }) {
  const disconnect = useDisconnectAgent();
  const [isOpen, setIsOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  // The table scrolls sideways, which would clip a menu inside it, so the
  // menu is placed against the window, under the button.
  const [place, setPlace] = useState<{ top: number; right: number } | null>(null);

  useEffect(() => {
    if (!isOpen) return;
    function onPointer(event: PointerEvent): void {
      if (!ref.current?.contains(event.target as Node)) setIsOpen(false);
    }
    function onKey(event: KeyboardEvent): void {
      if (event.key === 'Escape') setIsOpen(false);
    }
    function onMove(): void {
      setIsOpen(false);
    }
    window.addEventListener('pointerdown', onPointer);
    window.addEventListener('keydown', onKey);
    window.addEventListener('scroll', onMove, true);
    window.addEventListener('resize', onMove);
    return () => {
      window.removeEventListener('pointerdown', onPointer);
      window.removeEventListener('keydown', onKey);
      window.removeEventListener('scroll', onMove, true);
      window.removeEventListener('resize', onMove);
    };
  }, [isOpen]);

  return (
    <div ref={ref} className={styles.menuWrap}>
      <button
        type="button"
        className={styles.menuButton}
        aria-label={`More for ${agent.name}`}
        aria-haspopup="menu"
        aria-expanded={isOpen}
        onClick={(event) => {
          const box = event.currentTarget.getBoundingClientRect();
          setPlace({ top: box.bottom + 4, right: window.innerWidth - box.right });
          setIsOpen(!isOpen);
        }}
      >
        <MoreVertical size={16} aria-hidden="true" />
      </button>
      {isOpen && (
        <div
          className={styles.menu}
          role="menu"
          style={place ? { position: 'fixed', top: place.top, right: place.right } : undefined}
        >
          {onEditLimits && (
            <button
              type="button"
              role="menuitem"
              className={styles.item}
              onClick={() => {
                setIsOpen(false);
                onEditLimits();
              }}
            >
              Edit access
            </button>
          )}
          <button
            type="button"
            role="menuitem"
            className={styles.danger}
            disabled={disconnect.isPending}
            onClick={() => {
              disconnect.mutate(agent.id);
              setIsOpen(false);
            }}
          >
            Disconnect {agent.name}
          </button>
        </div>
      )}
    </div>
  );
}
