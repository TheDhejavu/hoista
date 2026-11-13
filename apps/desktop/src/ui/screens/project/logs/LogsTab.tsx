/**
 * Logs: recent lines from the app, its provider and its builds, newest first,
 * as a table. Open a line to see its details and what to ask your agent.
 * "Live" keeps reading new lines as they arrive.
 */
import { ChevronDown, Search } from 'lucide-react';
import { useState } from 'react';
import { useStoredFilter, useStoredOption } from '@ui/lib/storedChoice';
import type { LogLine, LogQuery, Resource } from '@shared/api';
import { useLogs, useProject } from '@ui/api/projects';
import { CopyPromptButton } from '@ui/components/AgentPrompt';
import { QueryState } from '@ui/components/QueryState';
import { SkeletonTableBody } from '@ui/components/Skeleton';
import { PillSelect, type Option } from '@ui/components/Select';
import { cx } from '@ui/lib/cx';
import { useRequiredParam } from '@ui/lib/useRequiredParam';
import { ProjectTab } from '../ProjectTab';
import { resourceKindLabel } from '../resourceKinds';
import styles from './LogsTab.module.css';
import { serviceName } from '@shared/providers';
import { ResourceSelect } from '@ui/components/ResourceSelect';

type SourceKind = LogLine['source'] | null;

/** `null` in a filter means "any". */
const RESOURCE_CHOICES: Option<SourceKind>[] = [
  { value: null, label: 'All' },
  { value: 'project', label: 'Project' },
  { value: 'request', label: 'Requests' },
  { value: 'provider', label: 'Provider events' },
  { value: 'build', label: 'Builds' },
];

const LEVEL_CHOICES: Option<LogLine['level'] | null>[] = [
  { value: null, label: 'Everything' },
  { value: 'error', label: 'Errors' },
  { value: 'warn', label: 'Warnings' },
  { value: 'info', label: 'Info' },
];

const MINUTES_PER_HOUR = 60;

/** How far back to look, in minutes. */
const TIME_CHOICES: Option<number>[] = [
  { value: MINUTES_PER_HOUR, label: 'Last hour' },
  { value: 24 * MINUTES_PER_HOUR, label: 'Last 24 hours' },
  { value: 7 * 24 * MINUTES_PER_HOUR, label: 'Last 7 days' },
];

/** `pinned` shows one resource's lines only, on that resource's page. */
export function LogsTab({ pinned }: { pinned?: string } = {}) {
  const projectId = useRequiredParam('projectId');
  const [search, setSearch] = useState('');
  // This project's filters stay as you left them.
  const [level, setLevel] = useStoredOption<LogLine['level'] | null>(
    `logs.${projectId}.level`,
    null,
    LEVEL_CHOICES,
  );
  const [source, setSource] = useStoredOption<SourceKind>(
    `logs.${projectId}.source`,
    null,
    RESOURCE_CHOICES,
  );
  // A day by default: a quiet service may log nothing for an hour.
  const [minutes, setMinutes] = useStoredOption<number>(
    `logs.${projectId}.since`,
    24 * MINUTES_PER_HOUR,
    TIME_CHOICES,
  );
  const [live, setLive] = useState(true);
  const [openKey, setOpenKey] = useState<string | null>(null);

  const query: LogQuery = {
    q: search,
    since_minutes: minutes,
    ...(level && { level }),
    ...(source && { source }),
    // On a resource's page, read only its provider.
    ...(pinned && { resource: pinned }),
  };
  const logs = useLogs(projectId, query, { live });
  const [chosenId, setResourceId] = useStoredFilter<string>(`logs.${projectId}.resource`, null);
  const resourceId = pinned ?? chosenId;
  const project = useProject(projectId).data;
  const resources = project?.resources ?? [];
  const every = [...(logs.data?.items ?? [])].reverse();
  const logResources = resources.filter((resource) =>
    every.some((line) => line.resource_id === resource.id),
  );
  const lines =
    resourceId === null ? every : every.filter((line) => line.resource_id === resourceId);
  // Open the newest error by default, so the panel shows something useful.
  const shownKey = openKey ?? keyOf(lines.find((line) => line.level === 'error'));
  const open = lines.find((line) => keyOf(line) === shownKey) ?? null;
  // Offer only the kinds of lines this project's providers have sent. Not every provider
  // reports build output or its own events; a filter that can't match isn't shown.
  const [seen, setSeen] = useState<ReadonlySet<LogLine['source']>>(new Set());
  const arrived = source === null ? lines.map((line) => line.source) : [];
  if (arrived.some((kind) => !seen.has(kind))) {
    // Remembered while a filter is on, so the list doesn't shrink to the one chosen.
    setSeen(new Set([...seen, ...arrived]));
  }
  const sourceChoices = RESOURCE_CHOICES.filter(
    (choice) => choice.value === null || choice.value === source || seen.has(choice.value),
  );

  return (
    <ProjectTab>
      <div className={cx(styles.layout, open && styles.withPanel)}>
        <div className={styles.main}>
          <div className={styles.toolbar}>
            <label className={styles.search}>
              <Search size={14} aria-hidden="true" />
              <input
                type="search"
                aria-label="Filter messages"
                placeholder="Filter messages"
                value={search}
                onChange={(event) => {
                  setSearch(event.target.value);
                }}
              />
            </label>
            {!pinned && logResources.length > 1 && (
              <ResourceSelect
                all
                resources={logResources}
                value={resourceId}
                onChange={setResourceId}
              />
            )}
            {sourceChoices.length > 2 && (
              <PillSelect
                label="Kind"
                value={source}
                options={sourceChoices}
                onChange={setSource}
              />
            )}
            <PillSelect label="Level" value={level} options={LEVEL_CHOICES} onChange={setLevel} />
            <PillSelect label="Time" value={minutes} options={TIME_CHOICES} onChange={setMinutes} />
            <button
              type="button"
              className={cx(styles.live, live && styles.liveOn)}
              aria-pressed={live}
              onClick={() => {
                setLive(!live);
              }}
            >
              <span className={styles.liveDot} />
              {live ? 'Live' : 'Paused'}
            </button>
          </div>

          <div className={styles.frame}>
            <QueryState
              query={logs}
              skeleton={
                <table className={styles.table}>
                  <LogsHead pinned={Boolean(pinned)} />
                  <SkeletonTableBody
                    count={8}
                    widths={pinned ? [70, 40, 60, '80%', null] : [70, 40, 80, 60, '80%', null]}
                    height={10}
                  />
                </table>
              }
            >
              {() =>
                lines.length === 0 ? (
                  <p className={styles.empty}>
                    {search || level || source
                      ? 'No log lines match these filters.'
                      : `Nothing logged in the ${TIME_CHOICES.find((choice) => choice.value === minutes)?.label.toLowerCase() ?? 'chosen time'}.${minutes < 7 * 24 * MINUTES_PER_HOUR ? ' Choose a longer time to look further back.' : ''}`}
                  </p>
                ) : (
                  <table className={styles.table}>
                    <LogsHead pinned={Boolean(pinned)} />
                    <tbody>
                      {lines.map((line) => {
                        const key = keyOf(line);
                        const isOpen = key === shownKey;
                        return (
                          <tr
                            key={key}
                            className={cx(isOpen && styles.openRow)}
                            aria-selected={isOpen}
                            onClick={() => {
                              setOpenKey(key);
                            }}
                          >
                            <td>{clock(line.at)}</td>
                            <td className={styles[line.level]}>{line.level}</td>
                            {!pinned && (
                              <td className={styles.faint}>
                                {line.status === null ? line.resource_name : resourceOf(line)}
                              </td>
                            )}
                            <td className={styles.strong}>
                              {line.status === null ? '' : line.what}
                            </td>
                            <td>{line.message}</td>
                            <td className={styles.chevron}>
                              {isOpen && <ChevronDown size={14} aria-hidden="true" />}
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                )
              }
            </QueryState>
          </div>
        </div>

        {open && (
          <LineDetail
            line={open}
            lines={lines}
            projectName={project?.project.name ?? projectId}
            resource={resources.find((resource) => resource.id === open.resource_id)}
          />
        )}
      </div>
    </ProjectTab>
  );
}

function LineDetail({
  line,
  lines,
  projectName,
  resource,
}: {
  line: LogLine;
  lines: LogLine[];
  projectName: string;
  resource: Resource | undefined;
}) {
  // Request lines have a request; provider runtime lines (Railway) only have text.
  const isRequest = line.status !== null;
  const same = lines.filter(
    (other) =>
      other.level === line.level &&
      other.resource_id === line.resource_id &&
      (isRequest
        ? other.what === line.what && other.status === line.status
        : other.message === line.message),
  ).length;
  const facts: [string, string][] = [
    ['Time', clock(line.at, true)],
    ['Resource', `${line.resource_name} · ${resourceOf(line)}`],
    ...(isRequest ? [['Request', line.what] as [string, string]] : []),
    ...(line.status ? [['Status', String(line.status)] as [string, string]] : []),
    ...line.facts.map((fact): [string, string] => [fact.label, fact.value]),
    ...(line.version ? [['Version', line.version] as [string, string]] : []),
    ...(line.level !== 'info' && same > 1
      ? [['Same line', `${String(same)}× shown`] as [string, string]]
      : []),
  ];
  const title = isRequest
    ? line.level === 'info'
      ? line.what
      : (line.message.split(' · ')[0] ?? line.message)
    : `${line.level === 'error' ? 'Error' : line.level === 'warn' ? 'Warning' : 'Log line'} from ${line.resource_name}`;
  // Exactly where the line came from, so the agent looks in the right place:
  // the app, the resource by name and Hoista id, its kind and its provider.
  const provider = resource ? serviceName(resource.provider) : null;
  const where = `${line.resource_name}${resource ? ` (${resourceKindLabel(resource.kind).toLowerCase()} on ${provider ?? ''}, Hoista resource id ${resource.id})` : ''} in the project ${projectName}`;
  const prompt = isRequest
    ? `In Hoista's logs for ${where}, ${line.what} returns ${line.message}. Find out why and fix it. Ask me before changing anything.`
    : `In Hoista's logs, ${where} logged this ${line.level === 'warn' ? 'warning' : 'error'} at ${line.at}:\n\n${line.message}\n\nRead the surrounding lines with get_log_history for resource ${line.resource_id}, find the cause and fix it. Ask me before changing anything.`;

  return (
    <aside className={styles.panel} aria-label="Log line">
      <div className={styles.panelHead}>
        <span className={cx(styles.dot, styles[`${line.level}Dot`])} />
        <b>{title}</b>
      </div>
      {!isRequest && <pre className={styles.lineText}>{line.message}</pre>}
      <dl className={styles.facts}>
        {facts.map(([term, value]) => (
          <div key={term}>
            <dt>{term}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
      {line.hint && <p className={styles.hint}>{line.hint}</p>}
      {line.level !== 'info' && (
        <div className={styles.fix}>
          <CopyPromptButton
            icon={false}
            strong
            label="Ask your agent to fix this"
            prompt={prompt}
          />
        </div>
      )}
    </aside>
  );
}

function keyOf(line: LogLine | undefined): string | null {
  return line ? `${line.at}-${line.resource_id}-${line.what}-${line.message.slice(0, 80)}` : null;
}

function resourceOf(line: LogLine): string {
  if (line.source === 'build' || !line.resource_kind) return 'build';
  return resourceKindLabel(line.resource_kind).toLowerCase();
}

/** `15:52:12`, or `15:52:12.418` in the detail panel. */
function clock(at: string, millis = false): string {
  const date = new Date(at);
  const time = date.toLocaleTimeString('en-GB', { hour12: false });
  return millis ? `${time}.${String(date.getMilliseconds()).padStart(3, '0')}` : time;
}

/** The log table's header, the same while lines load. */
function LogsHead({ pinned }: { pinned: boolean }) {
  return (
    <thead>
      <tr>
        <th scope="col">Time</th>
        <th scope="col">Level</th>
        {/* On a resource's own page every line is that resource's. */}
        {!pinned && <th scope="col">Resource</th>}
        <th scope="col">Request</th>
        <th scope="col">Message</th>
        <th scope="col">
          <span className="visually-hidden">Open</span>
        </th>
      </tr>
    </thead>
  );
}
