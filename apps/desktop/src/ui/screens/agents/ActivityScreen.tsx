/**
 * Activity: every action an agent took through Hoista, kept on this
 * computer, grouped by day and filterable. Open an action to see the
 * agent's reason, exactly what it sent and what happened next.
 */
import { useState } from 'react';
import { useStoredFilter, useStoredOption } from '@ui/lib/storedChoice';
import { Link, useSearchParams } from 'react-router-dom';
import type { AgentActivity } from '@shared/api';
import { useAgentActivity, useAgentCall, useAgents } from '@ui/api/agents';
import { useInEnvironment } from '@ui/lib/useInEnvironment';
import { AgentIcon } from '@ui/components/AgentIcon';
import { Button } from '@ui/components/Button';
import { QueryState } from '@ui/components/QueryState';
import { SkeletonRows } from '@ui/components/Skeleton';
import { PillSelect, type Option } from '@ui/components/Select';
import { cx } from '@ui/lib/cx';
import { plural } from '@ui/lib/format';
import { serviceName } from '@shared/providers';
import { reconnectPath } from '@shared/reconnect';
import styles from './ActivityScreen.module.css';
import { useProjectNames } from '@ui/lib/useProjectNames';

type Show = 'all' | 'changes' | 'reads';
type Result = 'all' | 'needs_you' | 'worked' | 'problems';

const SHOW_CHOICES: Option<Show>[] = [
  { value: 'all', label: 'Everything' },
  { value: 'changes', label: 'Changes' },
  { value: 'reads', label: 'Reads' },
];

const RESULT_CHOICES: Option<Result>[] = [
  { value: 'all', label: 'All' },
  { value: 'needs_you', label: 'Waiting for you' },
  { value: 'worked', label: 'Succeeded' },
  { value: 'problems', label: 'Failed or declined' },
];

const DAY = 24 * 60 * 60 * 1000;

/** How far back to show, in days. */
const WHEN_CHOICES: Option<number>[] = [
  { value: 1, label: 'Today' },
  { value: 7, label: 'Last 7 days' },
  { value: 30, label: 'Last 30 days' },
  { value: 90, label: 'Last 90 days' },
];

export function ActivityScreen() {
  const activity = useAgentActivity();
  const agents = useAgents();
  const projectName = useProjectNames();
  const inEnvironment = useInEnvironment();
  const [params, setParams] = useSearchParams();
  // Filters stay as you left them between visits and launches.
  const [agentId, setAgentId] = useStoredFilter<string>('activity.agent', null);
  const [projectId, setProjectId] = useStoredFilter<string>('activity.project', null);
  const [show, setShow] = useStoredOption<Show>('activity.show', 'all', SHOW_CHOICES);
  const [result, setResult] = useStoredOption<Result>('activity.result', 'all', RESULT_CHOICES);
  const [days, setDays] = useStoredOption<number>('activity.when', 7, WHEN_CHOICES);

  // Activity follows the environment the window shows, like the Projects list.
  const all = (activity.data?.items ?? []).filter((item) => inEnvironment(item.project_id));
  const projects = [...new Set(all.map((item) => item.project_id).filter((id) => id !== null))];
  // When the page opened; "Last 7 days" counts back from here.
  const [now] = useState(() => Date.now());
  const shown = all.filter(
    (item) =>
      (agentId === null || item.agent_id === agentId) &&
      (projectId === null || item.project_id === projectId) &&
      (show === 'all' || (show === 'reads') === item.read_only) &&
      matchesResult(item, result) &&
      now - Date.parse(item.at) < days * DAY,
  );
  // With nothing picked, open the newest action that changed something.
  const selected =
    shown.find((item) => item.id === params.get('item')) ??
    shown.find((item) => !item.read_only) ??
    null;

  function select(id: string): void {
    setParams({ item: id }, { replace: true });
  }

  return (
    <div className={styles.page}>
      <div className={styles.header}>
        <div>
          <div className={styles.trail}>
            <Link to="/agents">Agents</Link> / Activity
          </div>
          <h1>Activity</h1>
          <p>
            Every action an agent took through Hoista. Kept on this computer for 90 days. Chats with
            agents never reach Hoista.
          </p>
        </div>
        <Button
          large
          disabled={shown.length === 0}
          onClick={() => {
            exportActions(shown);
          }}
        >
          Export
        </Button>
      </div>

      <div className={styles.toolbar}>
        <PillSelect
          label="Agent"
          value={agentId}
          options={[
            { value: null, label: 'All' },
            ...(agents.data?.items ?? []).map((agent) => ({ value: agent.id, label: agent.name })),
          ]}
          onChange={setAgentId}
        />
        <PillSelect
          label="Project"
          value={projectId}
          options={[
            { value: null, label: 'All' },
            ...projects.map((id) => ({ value: id, label: projectName(id) })),
          ]}
          onChange={setProjectId}
        />
        <PillSelect label="Show" value={show} options={SHOW_CHOICES} onChange={setShow} />
        <PillSelect label="Result" value={result} options={RESULT_CHOICES} onChange={setResult} />
        <PillSelect label="When" value={days} options={WHEN_CHOICES} onChange={setDays} />
        <span className={styles.count}>{plural(shown.length, 'action')}</span>
      </div>

      <div className={cx(styles.layout, selected && styles.withPanel)}>
        <div className={styles.frame}>
          <QueryState query={activity} skeleton={<SkeletonRows count={6} columns={3} />}>
            {() =>
              shown.length === 0 ? (
                <p className={styles.empty}>No actions match these filters.</p>
              ) : (
                <table className={styles.table}>
                  <thead>
                    <tr>
                      <th scope="col">Time</th>
                      <th scope="col">Agent</th>
                      <th scope="col">Action</th>
                      <th scope="col">Project</th>
                      <th scope="col">Tool</th>
                      <th scope="col">Result</th>
                    </tr>
                  </thead>
                  {byDay(shown).map((day) => (
                    <tbody key={day.label}>
                      <tr className={styles.dayRow}>
                        <th scope="rowgroup" colSpan={6}>
                          {day.label}
                        </th>
                      </tr>
                      {day.items.map((item) => (
                        <tr
                          key={item.id}
                          className={cx(styles.row, item.id === selected?.id && styles.selected)}
                          aria-selected={item.id === selected?.id}
                          onClick={() => {
                            select(item.id);
                          }}
                        >
                          <td className={styles.time}>{clock(item.at)}</td>
                          <td>
                            <span className={styles.agent}>
                              <AgentIcon name={item.agent_name} small />
                              {item.agent_name}
                            </span>
                          </td>
                          <td className={item.read_only ? styles.muted : undefined}>
                            {item.summary}
                          </td>
                          <td className={cx(styles.muted, styles.nowrap)}>
                            {item.project_id ? projectName(item.project_id) : 'All projects'}
                          </td>
                          <td className={styles.tool}>{item.tool}</td>
                          <td>
                            <span className={styles.result}>
                              <span className={cx(styles.dot, styles[toneOf(item)])} />
                              {item.result}
                            </span>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  ))}
                </table>
              )
            }
          </QueryState>
        </div>
        {selected && <Detail item={selected} />}
      </div>
    </div>
  );
}

function Detail({ item }: { item: AgentActivity }) {
  const projectName = useProjectNames();
  const approved = item.result === 'You approved';
  const call = useAgentCall(item.id);
  const recorded = call.data;
  const sent = recorded?.sent ?? item.sent;
  const happened = item.happened.length > 0 ? item.happened : (recorded?.happened ?? []);

  return (
    <aside className={styles.panel} aria-label="Action">
      <div className={styles.panelTop}>
        <div className={styles.panelWho}>
          <AgentIcon name={item.agent_name} small />
          <b>{item.agent_name}</b>
          <span className={styles.panelWhen}>{dayAndTime(item.at)}</span>
        </div>
        <div className={styles.panelTitle}>{item.summary}</div>
        <div className={styles.muted}>
          {item.project_id ? projectName(item.project_id) : 'All projects'}
        </div>
      </div>
      <div className={styles.panelBody}>
        {item.reason && (
          <>
            <h3>Its reason</h3>
            <blockquote>“{item.reason}”</blockquote>
          </>
        )}
        {sent && (
          <>
            <h3>What it sent</h3>
            <pre className={styles.code}>{sent}</pre>
          </>
        )}
        <h3>Status</h3>
        <p className={styles.line}>
          <span className={cx(styles.dot, styles[statusTone(item, recorded?.status)])} />
          {recorded?.status ?? item.result}
        </p>
        {happened.length > 0 && (
          <>
            <h3>What happened</h3>
            {happened.map((line) => (
              <p key={line} className={styles.line}>
                <span className={cx(styles.dot, styles[toneOf(item)])} />
                {line}
              </p>
            ))}
          </>
        )}
        {approved && <p className={styles.note}>You allowed it when the agent asked.</p>}
        {item.waiting_on && (
          <Link to={reconnectPath(item.waiting_on)} className={styles.review}>
            Reconnect {serviceName(item.waiting_on.service)} →
          </Link>
        )}
      </div>
    </aside>
  );
}

function matchesResult(item: AgentActivity, result: Result): boolean {
  switch (result) {
    case 'all':
      return true;
    case 'needs_you':
      return item.waiting_on !== null && item.result === 'Waiting for you';
    case 'worked':
      return item.tone === 'good';
    case 'problems':
      return item.tone === 'bad';
  }
}

/** The status dot: what the recorded outcome says, else the row's own tone. */
function statusTone(
  item: AgentActivity,
  status: string | null | undefined,
): 'good' | 'bad' | 'warn' | 'neutral' {
  if (status === 'Refused' || status === 'You declined it') return 'bad';
  if (status === 'Waiting for your approval') return 'warn';
  return item.tone;
}

function toneOf(item: AgentActivity): 'good' | 'bad' | 'warn' | 'neutral' {
  return item.tone;
}

function byDay(items: AgentActivity[]): { label: string; items: AgentActivity[] }[] {
  const days: { label: string; items: AgentActivity[] }[] = [];
  for (const item of items) {
    const label = dayLabel(item.at);
    const last = days.at(-1);
    if (last?.label === label) last.items.push(item);
    else days.push({ label, items: [item] });
  }
  return days;
}

function dayLabel(at: string): string {
  const date = new Date(at);
  const today = new Date();
  const yesterday = new Date(today.getTime() - DAY);
  if (date.toDateString() === today.toDateString()) return 'Today';
  if (date.toDateString() === yesterday.toDateString()) return 'Yesterday';
  return date.toLocaleDateString('en-GB', { weekday: 'long', day: 'numeric', month: 'long' });
}

function clock(at: string): string {
  return new Date(at).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' });
}

function dayAndTime(at: string): string {
  return `${dayLabel(at)} ${clock(at)}`;
}

/** Saves the shown actions as a CSV file, for keeping records outside Hoista. */
function exportActions(items: AgentActivity[]): void {
  const quote = (value: string | null): string => `"${(value ?? '').replaceAll('"', '""')}"`;
  const rows = [
    ['time', 'agent', 'action', 'project', 'tool', 'result'].join(','),
    ...items.map((item) =>
      [item.at, item.agent_name, item.summary, item.project_id, item.tool, item.result]
        .map(quote)
        .join(','),
    ),
  ];
  const link = document.createElement('a');
  link.href = URL.createObjectURL(new Blob([rows.join('\n')], { type: 'text/csv' }));
  link.download = 'hoista-activity.csv';
  link.click();
  URL.revokeObjectURL(link.href);
}
