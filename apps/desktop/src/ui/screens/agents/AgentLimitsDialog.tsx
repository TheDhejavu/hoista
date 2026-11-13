/**
 * What one agent may do and touch: its access (View only, Ask for approval,
 * Full access), which Hoista environments it can reach (where production is
 * told apart from the rest, whatever each provider calls its own), which
 * projects, and when its access ends. Hoista checks all of it on every call.
 */
import { useState, type ReactNode } from 'react';
import type { AccessLevel, Agent, AgentException } from '@shared/api';
import { useProjects } from '@ui/api/projects';
import { useAccounts } from '@ui/api/accounts';
import { ALL_ENVIRONMENTS, useEnvironments } from '@ui/api/environments';
import { useUpdateAgent } from '@ui/api/agents';
import { AgentIcon } from '@ui/components/AgentIcon';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { MultiSelectField, SelectField, type Option } from '@ui/components/Select';
import { ACCESS_CHOICES, ACCESS_HINTS } from '@ui/lib/agentAccess';
import { plural, shortDate } from '@ui/lib/format';
import { serviceName } from '@shared/providers';
import { showToast } from '@ui/lib/toast';
import { ExceptionsSection, type Reachable } from './AgentExceptions';
import styles from './AgentLimitsDialog.module.css';
import { DAY_MS } from '@shared/time';

type Scope = 'all' | 'some';

/** Days until access ends; null is no end date, -1 keeps the current one. */
type Ends = number | null;

export function AgentLimitsDialog({ agent, onClose }: { agent: Agent; onClose: () => void }) {
  const projects = useProjects({ sort: 'name', environment: ALL_ENVIRONMENTS });
  const environmentList = useEnvironments().data ?? [];
  const update = useUpdateAgent();
  const accounts = useAccounts().data?.items ?? [];
  // Asking to save with nothing picked is when to say so, not before.
  const [tried, setTried] = useState(false);
  const [find, setFind] = useState('');

  const [projectScope, setProjectScope] = useState<Scope>(agent.projects === null ? 'all' : 'some');
  const [picked, setPicked] = useState<string[]>(agent.projects ?? []);
  // The Hoista environments it can reach; none saved means every one.
  const [reach, setReach] = useState<string[] | null>(agent.environment_ids ?? null);
  const [access, setAccess] = useState<AccessLevel>(agent.access);
  const [exceptions, setExceptions] = useState<AgentException[]>(agent.exceptions ?? []);
  const current = agent.access_ends_at ?? null;
  const [ends, setEnds] = useState<Ends>(current ? -1 : null);

  const endChoices: Option<Ends>[] = [
    ...(current ? [{ value: -1, label: `Keep: ends ${shortDate(current)}` }] : []),
    { value: null, label: 'No end date' },
    { value: 1, label: 'In 1 day' },
    { value: 7, label: 'In 7 days' },
    { value: 30, label: 'In 30 days' },
    { value: 90, label: 'In 90 days' },
  ];
  const reached = reach ?? environmentList.map((environment) => environment.id);
  const everyEnvironment = reach === null || reached.length >= environmentList.length;
  // Each choice narrows the next: projects in the environments it can reach,
  // then exceptions within the projects it can see.
  const inReach = (projects.data?.items ?? []).filter(
    (project) => everyEnvironment || reached.includes(project.environment_id),
  );
  const chosen = picked.filter((id) => inReach.some((project) => project.id === id));
  const reachable = projectScope === 'all' ? inReach : inReach.filter((p) => chosen.includes(p.id));
  const reachedNames = environmentList
    .filter((environment) => reached.includes(environment.id))
    .map((environment) => environment.name);
  const noProjects = projectScope === 'some' && chosen.length === 0;
  const query = find.trim().toLowerCase();
  const shownProjects = query
    ? inReach.filter((project) => project.name.toLowerCase().includes(query))
    : inReach;
  // The accounts its projects run on, with how many of them each one serves.
  const reachableNames = new Set(reachable.map((project) => project.name));
  const using = accounts
    .map((account) => ({
      account,
      count: account.used_by.filter((name) => reachableNames.has(name)).length,
    }))
    .filter((row) => row.count > 0);
  const noSpaces = environmentList.length > 0 && reached.length === 0;

  function toggle(list: string[], item: string): string[] {
    return list.includes(item) ? list.filter((value) => value !== item) : [...list, item];
  }

  function save(): void {
    if (noProjects) {
      setTried(true);
      return;
    }
    const endsAt = ends === -1 ? current : ends === null ? null : daysFromNow(ends);
    update.mutate(
      {
        agentId: agent.id,
        update: {
          // Projects outside its environments couldn't be reached anyway.
          projects: projectScope === 'all' ? null : chosen,
          // Every environment there is means every one, including later ones.
          environment_ids: reached.length >= environmentList.length ? null : reached,
          environment_rules: {},
          exceptions,
          access,
          access_ends_at: endsAt,
        },
      },
      {
        onSuccess: () => {
          showToast(`Saved ${agent.name}’s access`);
          onClose();
        },
      },
    );
  }

  // The live summary: what Save would give it, in sentences, as Cloudflare
  // shows a token before it's made.
  const where = (() => {
    const inEnvironments = everyEnvironment ? '' : ` in ${reachedNames.join(', ')}`;
    if (projectScope === 'all') return `every project${inEnvironments}`;
    const names = reachable.map((project) => project.name);
    if (names.length === 0) return 'no projects yet: pick at least one';
    const shown = names.slice(0, 3).join(', ');
    const more = names.length > 3 ? ` and ${names.length - 3} more` : '';
    return `${names.length} ${names.length === 1 ? 'project' : 'projects'} (${shown}${more})`;
  })();
  const endsLine =
    ends === -1 && current
      ? `Its access ends on ${shortDate(current)}.`
      : ends === null
        ? 'Its access has no end date.'
        : `Its access ends in ${ends} ${ends === 1 ? 'day' : 'days'}.`;

  return (
    <Dialog
      wide
      title={`${agent.name}’s access`}
      eyebrow="Agents"
      icon={<AgentIcon name={agent.name} small />}
      onClose={onClose}
      actions={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            busy={update.isPending}
            busyLabel="Saving"
            disabled={noSpaces}
            onClick={save}
          >
            Save
          </Button>
        </>
      }
    >
      <div className={styles.flow}>
        <div className={styles.steps}>
          <p className={styles.lead}>Top to bottom, each step can only narrow the one above it.</p>

          <Step
            number={1}
            title="Environments"
            hint="Projects in an environment it can’t reach are hidden from it."
          >
            <MultiSelectField
              label="Environments it can reach"
              values={reached}
              options={environmentList.map((environment) => ({
                value: environment.id,
                label: environment.name,
              }))}
              allLabel="Every environment"
              onChange={setReach}
            />
            <span className={noSpaces ? styles.warn : styles.hint}>
              {noSpaces
                ? 'Pick at least one environment.'
                : everyEnvironment
                  ? 'Includes environments you add later.'
                  : 'What it creates goes into the first one picked.'}
            </span>
          </Step>

          <Step
            number={2}
            title="Projects"
            hint={
              everyEnvironment
                ? 'From every environment.'
                : `Only projects in ${reachedNames.join(', ')}, from step 1.`
            }
          >
            <div
              className={styles.scopes}
              role="radiogroup"
              aria-label="Projects it can see and change"
            >
              <ScopeChoice
                on={projectScope === 'all'}
                title={`Every project${everyEnvironment ? '' : ` in ${reachedNames.join(', ')}`} (${inReach.length})`}
                hint="And ones you add later"
                onPick={() => {
                  setProjectScope('all');
                }}
              />
              <ScopeChoice
                on={projectScope === 'some'}
                title="Only the projects I pick"
                hint="It can’t see the rest, or act on a whole account"
                onPick={() => {
                  setProjectScope('some');
                }}
              />
            </div>
            {projectScope === 'some' &&
              (projects.isPending ? (
                <span className={styles.hint}>Loading projects</span>
              ) : inReach.length === 0 ? (
                <span className={styles.hint}>No projects in these environments yet.</span>
              ) : (
                <>
                  <div className={styles.findRow}>
                    <input
                      type="search"
                      className={styles.find}
                      placeholder="Find a project"
                      aria-label="Find a project"
                      value={find}
                      onChange={(event) => {
                        setFind(event.target.value);
                      }}
                    />
                    <span className={styles.count}>
                      {chosen.length} of {inReach.length} picked
                    </span>
                  </div>
                  {shownProjects.length === 0 ? (
                    <span className={styles.hint}>No project matches that.</span>
                  ) : (
                    <ul className={styles.projects} aria-label="Projects">
                      {shownProjects.map((project) => (
                        <li key={project.id}>
                          <label className={styles.project}>
                            <input
                              type="checkbox"
                              checked={picked.includes(project.id)}
                              onChange={() => {
                                setPicked(toggle(picked, project.id));
                                setTried(false);
                              }}
                            />
                            <span className={styles.projectName}>{project.name}</span>
                            {environmentList.length > 1 && (
                              <span className={styles.envTag}>
                                {environmentList.find((e) => e.id === project.environment_id)
                                  ?.name ?? project.environment_id}
                              </span>
                            )}
                            <span className={styles.logos}>
                              {project.providers.map((provider) => (
                                <span key={provider} title={serviceName(provider)}>
                                  <ServiceIcon id={provider} size={15} />
                                </span>
                              ))}
                            </span>
                          </label>
                        </li>
                      ))}
                    </ul>
                  )}
                </>
              ))}
            {tried && noProjects && (
              <span className={styles.warn} role="alert">
                Pick at least one project, or choose Every project.
              </span>
            )}
            {accounts.length > 0 && (
              <div className={styles.uses}>
                <div className={styles.usesHead}>Accounts it will use</div>
                {using.length === 0 ? (
                  <div className={styles.usesNone}>
                    {projectScope === 'some' && chosen.length === 0
                      ? 'Pick projects to see the accounts it will use.'
                      : 'None of these projects runs on a connected account yet.'}
                  </div>
                ) : (
                  using.map(({ account, count }) => (
                    <div key={account.id} className={styles.use}>
                      <ServiceIcon id={account.service} size={16} />
                      <b>{serviceName(account.service)}</b>
                      <span className={styles.useAccount}>{account.label}</span>
                      <span className={styles.useCount}>{plural(count, 'project')}</span>
                    </div>
                  ))
                )}
              </div>
            )}
          </Step>

          <Step number={3} title="What it can do" hint="In the projects from step 2.">
            <SelectField
              label="Access"
              value={access}
              options={ACCESS_CHOICES}
              onChange={setAccess}
            />
            <span className={access === 'full' ? styles.warn : styles.hint}>
              {ACCESS_HINTS[access]}.
            </span>
          </Step>

          <Step
            number={4}
            title="Exceptions"
            hint="Narrower access for some resources in those projects. Never more than step 3; the strictest one that applies wins."
          >
            <ExceptionsSection
              exceptions={exceptions}
              reachable={reachable}
              onChange={setExceptions}
            />
          </Step>

          <Step number={5} title="Access ends">
            <SelectField
              label="When its access ends"
              value={ends}
              options={endChoices}
              onChange={setEnds}
            />
            <span className={styles.hint}>
              After that every request is refused until you change this. Unused connections also end
              after the idle time in Settings.
            </span>
          </Step>
        </div>

        <section className={styles.summary} aria-live="polite" aria-label="What you’re saving">
          <h3 className={styles.summaryHead}>What you’re saving</h3>
          <ul className={styles.summaryList}>
            <li>
              {agent.name} {ACCESS_SENTENCE[access]} in {where}.
            </li>
            {exceptionLines(exceptions, reachable).map((line) => (
              <li key={line}>{line}</li>
            ))}
            <li>{endsLine}</li>
          </ul>
        </section>

        <ErrorMessage error={update.error} />
      </div>
    </Dialog>
  );
}

/** The moment `days` from now, as Hoista stores it. */
function daysFromNow(days: number): string {
  return new Date(Date.now() + days * DAY_MS).toISOString();
}

/** What each level lets it do, as part of the summary's first sentence. */
const ACCESS_SENTENCE: Record<AccessLevel, string> = {
  look: 'can only look',
  ask: 'can make changes and asks you before deleting, removing or connecting an account,',
  full: 'can change anything, including deleting, without asking,',
};

const GROUP_WORDS: Record<NonNullable<AgentException['group']>, string> = {
  services: 'Services',
  databases: 'Databases',
  storage: 'Storage',
  domains: 'Domains',
};

const EXCEPTION_WORDS: Record<AgentException['access'], string> = {
  ask: 'ask you first',
  look: 'are view only',
  hidden: 'are hidden from it',
};

/** The exceptions as summary lines; single resources are counted, not named. */
function exceptionLines(exceptions: AgentException[], reachable: Reachable[]): string[] {
  const lines: string[] = [];
  for (const exception of exceptions) {
    if (exception.group === null) continue;
    const inProject = exception.project_id
      ? reachable.find((project) => project.id === exception.project_id)?.name
      : null;
    if (exception.project_id && !inProject) continue;
    lines.push(
      `${GROUP_WORDS[exception.group]} ${EXCEPTION_WORDS[exception.access]} ${inProject ? `in ${inProject}` : 'everywhere it reaches'}.`,
    );
  }
  const single = exceptions.filter(
    (exception) =>
      exception.resource_id !== null &&
      reachable.some((project) => project.id === exception.project_id),
  );
  for (const level of ['hidden', 'look', 'ask'] as const) {
    const count = single.filter((exception) => exception.access === level).length;
    if (count > 0) {
      const what = `${count} single ${count === 1 ? 'resource' : 'resources'}`;
      lines.push(
        level === 'hidden'
          ? `${what} ${count === 1 ? 'is' : 'are'} hidden from it.`
          : level === 'look'
            ? `${what} ${count === 1 ? 'is' : 'are'} view only.`
            : `${what} ask you first.`,
      );
    }
  }
  return lines;
}

/** One of the two ways to choose its projects, as a card with a radio button. */
function ScopeChoice({
  on,
  title,
  hint,
  onPick,
}: {
  on: boolean;
  title: string;
  hint: string;
  onPick: () => void;
}) {
  return (
    <label className={on ? `${styles.scope} ${styles.scopeOn}` : styles.scope}>
      <input type="radio" name="project-scope" checked={on} onChange={onPick} />
      <span className={styles.scopeText}>
        <span className={styles.scopeTitle}>{title}</span>
        <span className={styles.hint}>{hint}</span>
      </span>
    </label>
  );
}

/** One numbered step of the flow. */
function Step({
  number,
  title,
  hint,
  children,
}: {
  number: number;
  title: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <section className={styles.step}>
      <span className={styles.number} aria-hidden="true">
        {number}
      </span>
      <div className={styles.stepBody}>
        <h3 className={styles.heading}>{title}</h3>
        {hint && <span className={styles.hint}>{hint}</span>}
        <div className={styles.fields}>{children}</div>
      </div>
    </section>
  );
}
