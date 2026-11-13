/**
 * A deploy that went live: a picture of the app with its address and
 * version, the checks it passed, and a note that the agent was told and how
 * to roll back if something is off.
 */
import { Check } from 'lucide-react';
import { useState } from 'react';
import { Link } from 'react-router-dom';
import type { Deploy, DeployStep } from '@shared/api';
import { useProject } from '@ui/api/projects';
import { useDeploys } from '@ui/api/deploys';
import { useProviderRead } from '@ui/api/providerOperations';
import { openExternal } from '@ui/api/system';
import { ActorIcon } from '@ui/components/AgentIcon';
import { ProjectShot } from '@ui/components/ProjectShot';
import { Button } from '@ui/components/Button';
import { formatDuration, hostOf } from '@ui/lib/format';
import { RowMenu } from '@ui/components/RowMenu';
import { showToast } from '@ui/lib/toast';
import { serviceName } from '@shared/providers';
import { canRollBackTo, changesUndoneBy, previousVersion } from '../project/deploys/describe';
import { RollBackDialog } from '../project/deploys/DeploysTab';
import { DeployHead, DeployPill } from './DeployResources';
import styles from './DeployDoneView.module.css';
import { isProviderDeploy, parseProviderDeploy } from '@shared/deployIds';

/** What each finished step proves, in the words of the Checks card. */
function checkOf(step: DeployStep): { label: string; value: string } | null {
  const took = step.duration_ms === null ? '' : formatDuration(step.duration_ms);
  switch (step.name) {
    case 'build':
      return { label: 'Built', value: took };
    case 'leak_check':
      return { label: 'No leaked keys in the bundle', value: 'checked' };
    case 'verify':
      return { label: step.detail ? `Health check · ${step.detail}` : 'Health check', value: took };
    case 'release':
      return { label: 'Released', value: took };
    default:
      return null;
  }
}

export function DeployDoneView({ deploy }: { deploy: Deploy }) {
  const project = useProject(deploy.project_id);
  const history = useDeploys(deploy.project_id).data?.items ?? [];
  const name = project.data?.project.name ?? deploy.project_id;
  const url = deploy.url ?? project.data?.project.url ?? null;
  const previous = deploy.current ? previousVersion(history) : null;
  const checks = deploy.steps
    .filter((step) => step.status === 'done')
    .map(checkOf)
    .filter((check) => check !== null);
  const who = deploy.started_by.kind === 'person' ? 'you' : deploy.started_by.name;
  // Roll back from here: to the version before, or to this one if it's earlier.
  const target = deploy.current ? previous : canRollBackTo(deploy) ? deploy : null;
  const [rolling, setRolling] = useState(false);
  const copy = (text: string, what: string) => {
    void navigator.clipboard.writeText(text).then(
      () => {
        showToast(`${what} copied`);
      },
      () => {
        showToast(`Couldn’t copy the ${what.toLowerCase()}`, { tone: 'error' });
      },
    );
  };

  return (
    <div className={styles.page}>
      <DeployHead
        deploy={deploy}
        title={`${name} is live`}
        mascot="done"
        pill={<DeployPill tone="good">Live · {deploy.version}</DeployPill>}
        sub={[
          deploy.duration_ms === null ? null : `Took ${formatDuration(deploy.duration_ms)}`,
          `by ${who}`,
        ]
          .filter(Boolean)
          .join(' · ')}
      />

      <div className={styles.layout}>
        <div className={styles.main}>
          <ProjectShot project={{ id: deploy.project_id, url }} height={330} />
          <div className={styles.address}>
            {url && (
              <span className={`mono ${styles.provider}`} title={hostOf(url)}>
                {hostOf(url)}
              </span>
            )}
            <span className={styles.pill}>
              {deploy.version} · {deploy.current ? 'in production' : 'earlier version'}
            </span>
            <span className={styles.actions}>
              {target && (
                <Button
                  large
                  onClick={() => {
                    setRolling(true);
                  }}
                >
                  {target === deploy
                    ? 'Roll back to this version'
                    : `Roll back to ${target.version}`}
                </Button>
              )}
              {url && (
                <Button
                  variant="accent"
                  large
                  onClick={() => {
                    void openExternal(url);
                  }}
                >
                  Open site ↗
                </Button>
              )}
              <RowMenu
                label={`More for ${deploy.version}`}
                items={[
                  ...(deploy.provider_page
                    ? [
                        {
                          label: `Open on ${deploy.provider ? serviceName(deploy.provider) : 'the provider'} ↗`,
                          onSelect: () => {
                            void openExternal(deploy.provider_page ?? '');
                          },
                        },
                      ]
                    : []),
                  ...(deploy.source.commit
                    ? [
                        {
                          label: 'Copy commit',
                          onSelect: () => {
                            copy(deploy.source.commit ?? '', 'Commit');
                          },
                        },
                      ]
                    : []),
                  {
                    label: 'Copy deploy ID',
                    onSelect: () => {
                      copy(deploy.id, 'Deploy ID');
                    },
                  },
                ]}
              />
            </span>
          </div>
        </div>
        <aside className={styles.side}>
          {isProviderDeploy(deploy.id) ? (
            <ProviderChecks deployId={deploy.id} />
          ) : (
            checks.length > 0 && <Checks checks={checks} />
          )}
          <div className={styles.told}>
            <div className={styles.toldHead}>
              <ActorIcon actor={deploy.started_by} />
              {deploy.started_by.kind === 'agent' ? (
                <>
                  <b>{deploy.started_by.name}</b> was told it’s live.
                </>
              ) : (
                <b>It’s live.</b>
              )}
            </div>
            <p>
              Every version is on the project’s{' '}
              <Link to={`/projects/${deploy.project_id}/deploys`}>Deploys tab</Link>.
            </p>
          </div>
        </aside>
      </div>
      {rolling && target && (
        <RollBackDialog
          projectName={name}
          providerName={deploy.provider ? serviceName(deploy.provider) : null}
          target={target}
          undone={changesUndoneBy(history, target)}
          onClose={() => {
            setRolling(false);
          }}
        />
      )}
    </div>
  );
}

function Checks({ checks }: { checks: { label: string; value: string; failed?: boolean }[] }) {
  return (
    <div className={styles.card}>
      <h2>Checks</h2>
      {checks.map((check) => (
        <div key={check.label} className={styles.check}>
          <span className={check.failed ? styles.cross : styles.tick}>
            {check.failed ? '×' : <Check size={11} strokeWidth={3.5} aria-hidden="true" />}
          </span>
          {check.label}
          <span className={styles.value}>{check.value}</span>
        </div>
      ))}
    </div>
  );
}

interface ProviderEvent {
  step: string | null;
  completedAt: string | null;
  payload: { durationMs: number | null; error: string | null; skipped: boolean | null } | null;
}

/** `BUILD_IMAGE` → `Build image`. */
function stepName(step: string): string {
  const words = step.toLowerCase().replace(/_/g, ' ');
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/**
 * A deploy at a provider: the steps the provider itself recorded (Railway's deploy
 * timeline), each with how long it took. Nothing recorded, no card.
 */
function ProviderChecks({ deployId }: { deployId: string }) {
  const { resourceId = '', deploymentId = '' } = parseProviderDeploy(deployId) ?? {};
  const events = useProviderRead<{ edges: { node: ProviderEvent }[] }>(
    { kind: 'resource', id: resourceId },
    'deploymentEvents',
    { id: deploymentId, first: 50 },
  );
  const steps = new Map<string, { label: string; value: string; failed?: boolean }>();
  for (const { node } of events.data?.edges ?? []) {
    if (!node.step) continue;
    const payload = node.payload;
    const took = payload?.durationMs ? formatDuration(payload.durationMs) : '';
    // The newest event for a step wins: a retried step shows how it ended.
    steps.set(node.step, {
      label: stepName(node.step),
      value: payload?.error
        ? 'failed'
        : payload?.skipped
          ? 'skipped'
          : took || (node.completedAt ? 'done' : 'running'),
      failed: Boolean(payload?.error),
    });
  }
  if (steps.size === 0) return null;
  return <Checks checks={[...steps.values()]} />;
}
