/**
 * Pieces every deploy page shares: the header with its trail, the row of
 * step chips, and the build output as a dark terminal.
 */
import { Check } from 'lucide-react';
import { Mascot, type MascotMood } from '@ui/components/Mascot';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Link } from 'react-router-dom';
import type { Deploy, DeployLogLine, DeployStep } from '@shared/api';
import { useDeployLog } from '@ui/api/deploys';
import { splitCommit } from '@ui/lib/commitMessage';
import { commitUrl } from '@ui/lib/links';
import { useProject } from '@ui/api/projects';
import { ExternalLink } from '@ui/components/ExternalLink';
import { serviceName } from '@shared/providers';
import { cx } from '@ui/lib/cx';
import { Skeleton } from '@ui/components/Skeleton';
import { stepLabel } from '@ui/lib/deploySteps';
import { elapsedClock } from '@ui/lib/format';
import styles from './DeployResources.module.css';
import { useProjectNames } from '@ui/lib/useProjectNames';

/**
 * A deploy's page as it will be, while the deploy loads: the trail and
 * title, the steps and the two panels, with placeholders for what's in them.
 */
export function DeploySkeleton() {
  return (
    <div className={styles.skeleton} role="status" aria-label="Loading">
      <header className={styles.head}>
        <div className={styles.trail}>
          <Link to="/deploys">Deploys</Link> /
        </div>
        <div className={styles.titleRow}>
          <Skeleton width={40} height={40} />
          <Skeleton width={280} height={28} />
          <Skeleton width={96} height={22} />
        </div>
        <Skeleton className={styles.subSkeleton} width={320} height={12} />
      </header>
      <ol className={styles.chips} aria-hidden="true">
        {Array.from({ length: 7 }, (_, step) => (
          <li key={step} className={cx(styles.chip, styles.chipSkeleton)}>
            <Skeleton width={70} height={12} />
            <Skeleton width={44} height={10} />
          </li>
        ))}
      </ol>
      <div className={styles.skeletonColumns}>
        <div className={styles.skeletonCard}>
          <Skeleton width={140} height={16} />
          <Skeleton width="80%" height={12} />
          <Skeleton width="60%" height={12} />
        </div>
        <div className={styles.skeletonCard}>
          {Array.from({ length: 6 }, (_, line) => (
            <Skeleton key={line} width={`${String(90 - line * 8)}%`} height={10} />
          ))}
        </div>
      </div>
    </div>
  );
}

export function DeployHead({
  deploy,
  title,
  pill,
  sub,
  actions,
  mascot,
}: {
  deploy: Deploy;
  title: string;
  pill: ReactNode;
  sub: string;
  actions?: ReactNode;
  /** The mascot's mood beside the title: deploying, done or failed. */
  mascot?: MascotMood;
}) {
  const projectName = useProjectNames();
  return (
    <header className={styles.head}>
      <div className={styles.trail}>
        <Link to="/deploys">Deploys</Link> / {projectName(deploy.project_id)}
      </div>
      <div className={styles.titleRow}>
        {mascot && <Mascot mood={mascot} size={40} />}
        <h1>{title}</h1>
        {pill}
        {actions && <div className={styles.actions}>{actions}</div>}
      </div>
      <p className={styles.sub}>{sub}</p>
      <CommitMessage message={deploy.source.message} />
      <DeployOut deploy={deploy} />
    </header>
  );
}

/** Where else the deploy can be seen: its commit on GitHub and its page at the provider. */
function DeployOut({ deploy }: { deploy: Deploy }) {
  const project = useProject(deploy.project_id);
  const resource = project.data?.resources.find((item) => item.id === deploy.resource_id);
  const commit = commitUrl(deploy.source.repo, deploy.source.commit);
  if (!commit && !deploy.provider_page) return null;
  return (
    <div className={styles.outLinks}>
      {commit && (
        <ExternalLink variant="text" url={commit}>
          Commit {deploy.source.commit?.slice(0, 7)} on GitHub
        </ExternalLink>
      )}
      {deploy.provider_page && (
        <ExternalLink variant="text" url={deploy.provider_page}>
          Open in {resource ? serviceName(resource.provider) : 'the provider'}
        </ExternalLink>
      )}
    </div>
  );
}

/** The commit a deploy shipped: its subject, and the rest behind Show more. */
function CommitMessage({ message }: { message: string | null | undefined }) {
  const [open, setOpen] = useState(false);
  const { subject, body } = splitCommit(message);
  if (!subject) return null;
  return (
    <div className={styles.commit}>
      <p className={styles.commitSubject}>{subject}</p>
      {body && open && <pre className={styles.commitBody}>{body}</pre>}
      {body && (
        <button
          type="button"
          className={styles.commitToggle}
          aria-expanded={open}
          onClick={() => {
            setOpen((was) => !was);
          }}
        >
          {open ? 'Show less' : 'Show more'}
        </button>
      )}
    </div>
  );
}

/** A status pill: `tone` colours the dot, and `bad` or `warn` tint the pill too. */
export function DeployPill({
  tone,
  children,
}: {
  tone: 'good' | 'bad' | 'warn' | 'off';
  children: ReactNode;
}) {
  return (
    <span
      className={cx(
        styles.pill,
        tone === 'bad' && styles.pillBad,
        tone === 'warn' && styles.pillWarn,
      )}
    >
      {tone !== 'off' && <span className={cx(styles.dot, styles[tone])} />}
      {children}
    </span>
  );
}

const STEP_WORDS: Record<DeployStep['status'], string> = {
  pending: 'waiting',
  running: 'working',
  done: 'done',
  failed: 'failed',
  skipped: 'waiting',
};

export function StepChips({ steps, stoppedAt }: { steps: DeployStep[]; stoppedAt?: string }) {
  return (
    <ol className={styles.chips} aria-label="Deploy steps">
      {steps.map((step) => {
        const stopped = step.name === stoppedAt;
        return (
          <li
            key={step.name}
            className={cx(
              styles.chip,
              (step.status === 'pending' || step.status === 'skipped') &&
                !stopped &&
                styles.waiting,
              stopped && styles.stopped,
              step.status === 'running' && styles.running,
              step.status === 'failed' && styles.failed,
            )}
          >
            <span className={styles.chipName}>
              <span className={styles.mark}>
                {step.status === 'done' && <Check size={10} strokeWidth={3.5} aria-hidden="true" />}
              </span>
              {stepLabel(step.name)}
            </span>
            <span className={styles.chipState}>
              {stopped ? 'stopped' : STEP_WORDS[step.status]}
            </span>
          </li>
        );
      })}
    </ol>
  );
}

const LINE_STYLES: Record<DeployLogLine['level'], string | undefined> = {
  info: undefined,
  ok: styles.logOk,
  warn: styles.logWarn,
  error: styles.logError,
};

/** The deploy's output in a dark terminal, times counted from the start; it follows new lines while live. */
export function DeployLog({
  deployId,
  height,
  live = false,
}: {
  deployId: string;
  height?: number;
  live?: boolean;
}) {
  const log = useDeployLog(deployId, live);
  const lines = log.data?.lines ?? [];
  const box = useRef<HTMLDivElement>(null);
  // Follow the newest line while it runs, as a terminal does.
  useEffect(() => {
    if (live && box.current) box.current.scrollTop = box.current.scrollHeight;
  }, [live, lines.length]);

  return (
    <div
      className={styles.terminal}
      role="log"
      aria-live={live ? 'polite' : undefined}
      style={height ? { minHeight: height } : undefined}
      ref={box}
    >
      {lines.length === 0 && <span className={styles.time}>Waiting for output</span>}
      {lines.map((line, index) => (
        <div key={index} className={styles.line}>
          <span className={styles.time}>{elapsedClock(line.offset_ms)}</span>
          <span className={LINE_STYLES[line.level]}>{line.text}</span>
        </div>
      ))}
    </div>
  );
}
