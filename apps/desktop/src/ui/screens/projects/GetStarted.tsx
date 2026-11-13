/**
 * The Get started checklist beside the Projects screen: connect a provider (Hoista
 * finds the projects on it), then add the projects you want to watch. Each step ticks
 * itself off from what Hoista can see. Using an agent is offered underneath as
 * an option, with a prompt that covers looking after the projects, not only
 * deploying them. Once both steps are done the card can be hidden.
 */
import { Check } from 'lucide-react';
import { Link } from 'react-router-dom';
import { useAccounts } from '@ui/api/accounts';
import { useAgents } from '@ui/api/agents';
import { useProjects } from '@ui/api/projects';
import { CopyPromptButton } from '@ui/components/AgentPrompt';
import { cx } from '@ui/lib/cx';
import styles from './GetStarted.module.css';

const STEPS = [
  { title: 'Connect a provider', text: 'Hoista finds your projects on it' },
  { title: 'Add your projects', text: 'Pick the ones Hoista watches' },
];

export const SETUP_STEPS = STEPS.length;

/** What to paste into an agent so it looks after the projects through Hoista. */
export const AGENT_PROMPT = `Hoista is connected to you as an MCP server ("hoista"). It holds my access to my providers, so use Hoista's tools for everything about my projects. Don't use provider CLIs, dashboards or direct API calls.

Start by getting me up to speed:
1. Call get_my_access to see which providers are connected and what Hoista can do on each. Then call list_apps and tell me what I have: where each project runs, whether it's live, its public URL, and what each provider reports as used so far this period (get_bills). Don't guess costs per project.
2. Call get_attention and tell me anything that needs me, most important first.
3. Call list_import_candidates and list the projects on my providers that I haven't added yet. Ask me before adding any with import_resources.

After that, when I ask about a project:
- Details: get_app, then get_resource for a resource's latest usage.
- Logs: get_log_history for a time range, get_logs for what's happening right now. Quote the lines that matter, with their times.
- Usage: get_usage for a project since the start of the month, and get_bills for what each provider reports as used so far.
- If something is broken, find the cause in the logs before changing anything, and tell me in one sentence what you found.
- Only plan or deploy when I ask you to.

Rules: never run a provider's own CLI; never ask me for a token or secret in chat; if Hoista says it can't do something on a provider, tell me plainly instead of working around it. Actions that need my approval wait for me in the Hoista app.`;

/** How many steps are done, 0–2. */
export function useSetupDone(): number {
  const accounts = useAccounts().data?.items.length ?? 0;
  const projects = useProjects({}).data?.totals.projects ?? 0;
  if (accounts === 0) return 0;
  return projects === 0 ? 1 : 2;
}

export function GetStarted({
  onConnectProvider,
  onHide,
}: {
  onConnectProvider?: () => void;
  /** Shown once every step is done. */
  onHide?: () => void;
}) {
  const done = useSetupDone();
  const agents = useAgents().data?.items.length ?? 0;

  return (
    <aside className={styles.card} aria-labelledby="get-started-heading">
      <div className={styles.head}>
        <h2 id="get-started-heading">Get started</h2>
        <span className={styles.count}>
          {done} of {SETUP_STEPS}
        </span>
      </div>
      <div className={styles.bars} aria-hidden="true">
        {STEPS.map((step, index) => (
          <i key={step.title} className={cx(index < done && styles.on)} />
        ))}
      </div>
      {STEPS.map((step, index) => {
        const state = index < done ? 'done' : index === done ? 'now' : 'later';
        const body = (
          <>
            <span className={cx(styles.badge, styles[state])}>
              {state === 'done' ? <Check size={12} strokeWidth={3.5} /> : index + 1}
            </span>
            <span className={styles.text}>
              <span className={styles.title}>{step.title}</span>
              <span className={styles.sub}>{step.text}</span>
            </span>
            {state === 'now' && <span className={styles.start}>Start</span>}
          </>
        );
        if (index === 0 && !onConnectProvider) {
          return (
            <Link key={step.title} to="/accounts" className={styles.step}>
              {body}
            </Link>
          );
        }
        return (
          <button
            key={step.title}
            type="button"
            className={styles.step}
            onClick={() => {
              if (index === 0) onConnectProvider?.();
              else document.getElementById('found-title')?.scrollIntoView({ behavior: 'smooth' });
            }}
          >
            {body}
          </button>
        );
      })}

      <div className={styles.agent}>
        <span className={styles.optional}>Optional</span>
        <span className={styles.title}>Use it from your agent</span>
        <span className={styles.sub}>
          Claude Code, Cursor and others can check status, read logs and bills, and bring in
          projects through Hoista.
        </span>
        <div className={styles.agentActions}>
          {agents === 0 && (
            <Link to="/agents?connect=1" className={styles.agentLink}>
              Connect an agent
            </Link>
          )}
          <CopyPromptButton icon={false} label="Copy starter prompt" prompt={AGENT_PROMPT} />
        </div>
      </div>

      {done === SETUP_STEPS && onHide && (
        <button type="button" className={styles.hide} onClick={onHide}>
          Hide this
        </button>
      )}
    </aside>
  );
}
