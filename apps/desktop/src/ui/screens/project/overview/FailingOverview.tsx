/**
 * The overview of a project whose recent deploys failed: what went wrong and
 * that the live site is untouched, the version serving now, the recent
 * deploys, and the fix right here, such as the missing key.
 */
import { KeyRound } from 'lucide-react';
import { useRef, useState, type ReactNode, type RefObject } from 'react';
import { useNavigate } from 'react-router-dom';
import type { ProjectDetail, Deploy, StepName } from '@shared/api';
import { serviceName } from '@shared/providers';
import { useSetSecret } from '@ui/api/projects';
import { useDeploys } from '@ui/api/deploys';
import { ActorIcon } from '@ui/components/AgentIcon';
import { CopyPromptButton } from '@ui/components/AgentPrompt';
import { Button } from '@ui/components/Button';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { cx } from '@ui/lib/cx';
import { formatCount, timeAgo } from '@ui/lib/format';
import styles from './FailingOverview.module.css';
import cards from './ProjectOverview.module.css';
import { useProjectNames } from '@ui/lib/useProjectNames';
import { failingDeploys } from '../deploys/describe';

const STEP_WORDS: Record<StepName, string> = {
  detect: 'detect',
  plan: 'plan',
  secrets: 'secrets',
  leak_check: 'secret scan',
  build: 'build',
  release: 'release',
  verify: 'check',
};

/** True when a resource's newest deploy failed, which is when this overview applies. */
export function lastDeployFailed(deploys: Deploy[]): boolean {
  return failingDeploys(deploys) !== null;
}

/** `children` is the project's resources, kept on the left as on the usual overview. */
export function FailingOverview({
  detail,
  children,
}: {
  detail: ProjectDetail;
  children?: ReactNode;
}) {
  const { project, stats } = detail;
  // Only the failing resource's own deploys: another service's live version isn't this one's.
  const deploys = failingDeploys(useDeploys(project.id).data?.items ?? []) ?? [];
  const serving = deploys.find((deploy) => deploy.current) ?? null;
  const failedRun = takeWhile(deploys, (deploy) => deploy.status === 'failed');
  const latest = failedRun[0];
  const needed = latest?.failure?.missing_secrets?.[0] ?? null;
  const keyField = useRef<HTMLInputElement>(null);
  // Saved here just now: the banner says it's set and what's left to do.
  const [saved, setSaved] = useState<string | null>(null);

  if (!latest) return null;
  const missing = needed !== null && needed !== saved ? needed : null;
  const step = latest.failure?.step ? STEP_WORDS[latest.failure.step] : null;
  const count = failedRun.length;
  const provider = project.providers[0] ? serviceName(project.providers[0]) : 'your provider';
  // A provider's own deploy (Railway building from a push): Hoista didn't run it, so
  // it can't say which step stopped or that the live site is unchanged.
  const byProvider = latest.started_by.kind === 'system';
  // With several resources, say which one failed.
  const failedName =
    latest.resource_id && detail.resources.length > 1
      ? detail.resources.find((resource) => resource.id === latest.resource_id)?.name
      : undefined;
  const of = failedName ? ` of ${failedName}` : '';
  const stopped =
    count > 1 ? `The last ${countWord(count)} deploys${of} failed` : `The last deploy${of} failed`;
  const headline = byProvider
    ? `${stopped} on ${provider}`
    : step
      ? `${count > 1 ? `The last ${countWord(count)} deploys stopped` : 'The last deploy stopped'} at the ${step} step`
      : stopped;

  return (
    <div className={cards.page}>
      <div className={styles.banner} role="alert">
        <span className={styles.bannerIcon}>
          <KeyRound size={14} aria-hidden="true" />
        </span>
        <div className={styles.bannerText}>
          <b>{saved ? `${saved} is set` : headline}</b>
          {saved ? (
            <span>
              {provider} has it now. Deploy again to put the fix live;{' '}
              {serving ? `${serving.version} keeps serving until then.` : 'nothing is live yet.'}
            </span>
          ) : (
            <span>
              {missing ? (
                <>
                  The code reads <span className="mono">{missing}</span> and it isn’t set.{' '}
                </>
              ) : (
                `${latest.failure?.message ?? ''} `
              )}
              {byProvider
                ? ''
                : serving
                  ? `Production is unchanged and still runs ${serving.version}.`
                  : 'Nothing was released.'}
            </span>
          )}
        </div>
        {saved && (
          <CopyPromptButton
            strong
            label="Ask your agent to deploy again"
            prompt={`Deploy ${project.name} again with Hoista`}
          />
        )}
        {missing && (
          <Button
            variant="accent"
            large
            onClick={() => {
              keyField.current?.focus();
            }}
          >
            Add the key
          </Button>
        )}
      </div>

      <div className={cards.stats}>
        <div className={cards.card}>
          <div className={cards.cardLabel}>Live version</div>
          <div className={cards.cardValue}>{serving?.version ?? 'Nothing'}</div>
          {serving && <div className={cards.small}>deployed {timeAgo(serving.created_at)}</div>}
        </div>
        <div className={cards.card}>
          <div className={cards.cardLabel}>Requests · 7 days</div>
          <div className={cards.cardValue}>
            {stats.requests === null ? '—' : formatCount(stats.requests)}
          </div>
        </div>
        <div className={cards.card}>
          <div className={cards.cardLabel}>Last attempt</div>
          <div className={cards.deploy}>
            <ActorIcon actor={latest.started_by} />
            <span>{timeAgo(latest.created_at)}</span>
          </div>
        </div>
      </div>

      <div className={cards.columns}>
        {children}
        <div className={styles.side}>
          {(missing ?? saved) && (
            <section aria-labelledby="fix-heading">
              <h2 id="fix-heading">Fix it here</h2>
              <KeyForm
                projectId={project.id}
                name={missing ?? saved ?? ''}
                provider={provider}
                inputRef={keyField}
                onSaved={setSaved}
              />
            </section>
          )}
          <section aria-labelledby="recent-heading">
            <h2 id="recent-heading">Recent deploys</h2>
            <RecentList deploys={deploys.slice(0, 3)} />
          </section>
        </div>
      </div>
    </div>
  );
}

function RecentList({ deploys }: { deploys: Deploy[] }) {
  const navigate = useNavigate();
  return (
    <div className={cards.table}>
      {deploys.map((deploy) => (
        <button
          key={deploy.id}
          type="button"
          className={cx(cards.row, styles.deployRow)}
          onClick={() => {
            void navigate(`/deploys/${deploy.id}`);
          }}
        >
          <span className={cx(cards.dot, deploy.status === 'failed' ? cards.bad : cards.good)} />
          <span className={styles.deployText}>
            <span>{deploy.source.message ?? deploy.version}</span>
            <span className={cx('mono', styles.deployMeta)}>
              {[
                deploy.source.ref,
                deploy.status === 'failed' && deploy.failure?.step
                  ? `stopped at ${STEP_WORDS[deploy.failure.step]}`
                  : null,
                deploy.current ? `${deploy.version} · in production` : null,
              ]
                .filter(Boolean)
                .join(' · ')}
            </span>
          </span>
          <ActorIcon actor={deploy.started_by} />
          <span className={cards.muted}>{timeAgo(deploy.created_at)}</span>
        </button>
      ))}
    </div>
  );
}

function KeyForm({
  projectId,
  name,
  provider,
  inputRef,
  onSaved,
}: {
  projectId: string;
  name: string;
  provider: string;
  inputRef: RefObject<HTMLInputElement | null>;
  onSaved: (name: string) => void;
}) {
  const [value, setValue] = useState('');
  const save = useSetSecret(projectId);
  const projectName = useProjectNames();
  const inputId = `secret-${name}`;

  return (
    <form
      className={styles.form}
      onSubmit={(event) => {
        event.preventDefault();
        if (!value) return;
        save.mutate(
          { name, value },
          {
            onSuccess: () => {
              setValue('');
              onSaved(name);
            },
          },
        );
      }}
    >
      <label htmlFor={inputId} className={styles.keyName}>
        {name}
      </label>
      <input
        ref={inputRef}
        id={inputId}
        type="password"
        autoComplete="off"
        spellCheck={false}
        className={cx('mono', styles.input)}
        placeholder={placeholderFor(name)}
        value={value}
        onChange={(event) => {
          setValue(event.target.value);
        }}
      />
      <p className={styles.hint}>
        Sent straight to {provider} as a secret. Never stored here, never shown to your agent.
      </p>
      {save.error && <ErrorMessage error={save.error} />}
      <div className={styles.formActions}>
        <Button type="submit" variant="accent" large disabled={save.isPending}>
          {save.isSuccess ? 'Key saved' : 'Save key'}
        </Button>
        <CopyPromptButton
          icon={false}
          label="Then ask your agent to deploy again"
          prompt={`Deploy ${projectName(projectId)} again with Hoista`}
        />
      </div>
    </form>
  );
}

/** The usual start of a key, so it's clear which one goes here. */
function placeholderFor(name: string): string {
  if (name.startsWith('RESEND')) return 're_…';
  if (name.startsWith('STRIPE')) return 'sk_…';
  if (name.startsWith('OPENAI')) return 'sk-…';
  return 'Paste the value';
}

function countWord(count: number): string {
  return ['zero', 'one', 'two', 'three', 'four', 'five'][count] ?? String(count);
}

function takeWhile<T>(items: T[], keep: (item: T) => boolean): T[] {
  const out: T[] = [];
  for (const item of items) {
    if (!keep(item)) break;
    out.push(item);
  }
  return out;
}
