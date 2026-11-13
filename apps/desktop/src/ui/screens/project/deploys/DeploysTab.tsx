/**
 * Deploys: every deploy of the app, newest first, with who started it, how
 * long it took and where it stands. Any earlier version that went live can
 * be put back in front of traffic; nothing is rebuilt.
 */
import { serviceName } from '@shared/providers';
import { runsCode } from '../resourceKinds';
import { Fragment, useState } from 'react';
import { GitBranch } from 'lucide-react';
import { useStoredFilter, useStoredOption } from '@ui/lib/storedChoice';
import { Link } from 'react-router-dom';
import { splitCommit } from '@ui/lib/commitMessage';
import { commitUrl } from '@ui/lib/links';
import { ExternalLink } from '@ui/components/ExternalLink';
import { openExternal } from '@ui/api/system';
import type { Deploy, DeployStatus } from '@shared/api';
import { useProject } from '@ui/api/projects';
import { useDeploys, useProviderAction, useRollBack } from '@ui/api/deploys';
import { useRunProviderOperation } from '@ui/api/providerOperations';
import { ActorIcon } from '@ui/components/AgentIcon';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { Panel } from '@ui/components/Panel';
import { QueryState } from '@ui/components/QueryState';
import { SkeletonRows } from '@ui/components/Skeleton';
import { PillSelect, type Option } from '@ui/components/Select';
import { cx } from '@ui/lib/cx';
import { formatDuration, timeAgo } from '@ui/lib/format';
import { useRequiredParam } from '@ui/lib/useRequiredParam';
import { ProjectTab, tabStyles } from '../ProjectTab';
import { canRollBackTo, changeTitle, changesUndoneBy } from './describe';
import { DeploySteps } from './DeploySteps';
import { MoreLines } from './MoreLines';
import styles from './DeploysTab.module.css';
import { ResourceSelect } from '@ui/components/ResourceSelect';
import { RowMenu, type RowMenuItem } from '@ui/components/RowMenu';
import { CopyField } from '@ui/components/CopyField';
import { useHoistCommand } from '@ui/lib/hoistCommand';
import { showToast } from '@ui/lib/toast';
import { isProviderDeploy, parseProviderDeploy } from '@shared/deployIds';

type StatusFilter = 'live' | 'failed' | 'cancelled' | null;

const STATUS_CHOICES: Option<StatusFilter>[] = [
  { value: null, label: 'All' },
  { value: 'live', label: 'Live' },
  { value: 'failed', label: 'Failed' },
  { value: 'cancelled', label: 'Stopped' },
];

/** `pinned` shows one resource's deploys only, on that resource's page. */
export function DeploysTab({ pinned }: { pinned?: string } = {}) {
  const projectId = useRequiredParam('projectId');
  const deploys = useDeploys(projectId);
  const detail = useProject(projectId).data;
  const project = detail?.project;
  // The resource that runs code, for provider actions that aren't about one deploy.
  const codeResource =
    project?.mode === 'watching'
      ? (detail?.resources.find((resource) => runsCode(resource.kind)) ?? null)
      : null;
  const deployLatest = useProviderAction();
  const projectName = project?.name ?? projectId;
  // The provider whose deploy history this is: the one running the project's code.
  const providerName =
    (codeResource ? serviceName(codeResource.provider) : null) ??
    (project?.runs_on.length === 1 ? project.runs_on[0] : undefined) ??
    'its provider';
  // This project's filters stay as you left them.
  const [by, setBy] = useStoredFilter<string>(`deploys.${projectId}.by`, null);
  const [status, setStatus] = useStoredOption<StatusFilter>(
    `deploys.${projectId}.status`,
    null,
    STATUS_CHOICES,
  );
  const [chosenResource, setResourceId] = useStoredFilter<string>(
    `deploys.${projectId}.resource`,
    null,
  );
  const resourceId = pinned ?? chosenResource;

  const all = deploys.data?.items ?? [];
  const people = [...new Set(all.map((deploy) => shortName(deploy)))];
  const deployResources = (detail?.resources ?? []).filter((resource) =>
    all.some((deploy) => deploy.resource_id === resource.id),
  );
  const shown = all.filter(
    (deploy) =>
      (by === null || shortName(deploy) === by) &&
      (status === null || deploy.status === status) &&
      (resourceId === null || deploy.resource_id === resourceId),
  );

  return (
    <ProjectTab>
      <div className={tabStyles.intro}>
        <div className={tabStyles.controls}>
          {!pinned && deployResources.length > 1 && (
            <ResourceSelect
              all
              resources={deployResources}
              value={resourceId}
              onChange={setResourceId}
            />
          )}
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
        </div>
        {codeResource && project?.source.kind === 'github' && (
          <Button
            variant="accent"
            busy={deployLatest.isPending}
            busyLabel="Starting the deploy"
            onClick={() => {
              deployLatest.mutate({ resourceId: codeResource.id, action: 'deploy_latest' });
            }}
          >
            Deploy newest commit
          </Button>
        )}
      </div>
      <p className={styles.note}>
        {project?.mode === 'watching'
          ? `${providerName}’s own deploy history. Redeploy, restart or roll back from a row’s menu, and Hoista asks ${providerName} to do it.`
          : 'Rolling back only switches which version serves traffic. Nothing is rebuilt.'}
      </p>

      <ErrorMessage error={deployLatest.error} />
      <Panel className={tabStyles.tableWrap}>
        <QueryState query={deploys} skeleton={<SkeletonRows count={4} columns={5} />}>
          {() =>
            all.length === 0 ? (
              <NoDeploys
                projectName={projectName}
                managed={project?.mode === 'managed'}
                providerName={providerName}
              />
            ) : (
              <DeploysTable
                deploys={shown}
                all={all}
                projectName={projectName}
                providerName={providerName}
              />
            )
          }
        </QueryState>
      </Panel>
    </ProjectTab>
  );
}

/** No deploys yet: how to make the first one, from the terminal. */
function NoDeploys({
  projectName,
  managed,
  providerName,
}: {
  projectName: string;
  managed: boolean;
  providerName: string;
}) {
  const command = useHoistCommand();
  if (!managed) {
    return (
      <p className={styles.empty}>
        No deploys yet. Deploys made at {providerName} show up here as they happen.
      </p>
    );
  }
  return (
    <div className={styles.noDeploys}>
      <p className={styles.noDeploysText}>
        No deploys yet. From the project’s folder, deploy it with:
      </p>
      <CopyField text={command(['deploy', '--project', projectName])} label="deploy command" />
    </div>
  );
}

/** "You" rather than "You in Hoista Desktop", as the table shows it. */
function shortName(deploy: Deploy): string {
  return deploy.started_by.kind === 'person' ? 'You' : deploy.started_by.name;
}

const WAITING = new Set(['NEEDS_APPROVAL', 'WAITING']);
const UNDER_WAY = new Set(['BUILDING', 'DEPLOYING', 'QUEUED', 'INITIALIZING']);
const FINISHED = new Set(['SUCCESS', 'FAILED', 'CRASHED', 'SKIPPED', 'SLEEPING']);

/**
 * What can be done to one deploy at the provider: approve one waiting for it,
 * cancel one under way, or remove an old one (that asks first).
 */
function DeployMenu({
  deploy,
  providerAction,
  onRollBack,
}: {
  deploy: Deploy;
  providerAction: ReturnType<typeof useProviderAction>;
  onRollBack: () => void;
}) {
  const { resourceId = '', deploymentId = '' } = parseProviderDeploy(deploy.id) ?? {};
  const run = useRunProviderOperation({ kind: 'resource', id: resourceId });
  const [removing, setRemoving] = useState(false);
  const status = deploy.provider_status ?? '';
  const busy = (name: string) => run.isPending && run.variables.name === name;
  const go = (name: string) => () => {
    run.mutate({ name, args: { id: deploymentId } });
  };
  const acting =
    deploy.current &&
    providerAction.isPending &&
    providerAction.variables.resourceId === resourceId;
  const working = acting
    ? providerAction.variables.action === 'redeploy'
      ? 'Redeploying'
      : 'Restarting'
    : busy('deploymentApprove')
      ? 'Approving'
      : busy('deploymentCancel')
        ? 'Cancelling'
        : null;
  const command = useHoistCommand();
  const copyItem = (label: string, text: string, said: string): RowMenuItem => ({
    label,
    onSelect: () => {
      void navigator.clipboard.writeText(text).then(() => {
        showToast(said);
      });
    },
  });
  const items: RowMenuItem[] = [
    ...(deploy.current && resourceId
      ? (['redeploy', 'restart'] as const).map((action) => ({
          label: action === 'redeploy' ? 'Redeploy' : 'Restart',
          disabled: providerAction.isPending,
          onSelect: () => {
            providerAction.mutate({ resourceId, action });
          },
        }))
      : []),
    ...(canRollBackTo(deploy) ? [{ label: 'Roll back to this', onSelect: onRollBack }] : []),
    ...(resourceId && WAITING.has(status)
      ? [{ label: 'Approve', disabled: run.isPending, onSelect: go('deploymentApprove') }]
      : []),
    ...(resourceId && (UNDER_WAY.has(status) || WAITING.has(status))
      ? [{ label: 'Cancel', disabled: run.isPending, onSelect: go('deploymentCancel') }]
      : []),
    copyItem('Copy deploy ID', deploy.id, `Copied the ID of ${deploy.version}`),
    copyItem(
      'Copy command',
      command(['deploys', 'show', deploy.id, '--log']),
      `Copied the command for ${deploy.version}`,
    ),
    ...(resourceId && !deploy.current && FINISHED.has(status)
      ? [
          {
            label: 'Remove',
            danger: true,
            onSelect: () => {
              setRemoving(true);
            },
          },
        ]
      : []),
  ];
  return (
    // The row opens its steps on click; the menu and its dialog don't.
    <span
      className={styles.rowActions}
      onClick={(event) => {
        event.stopPropagation();
      }}
    >
      {working && (
        <span className={styles.working} role="status">
          <span className={styles.spinner} aria-hidden="true" />
          {working}
        </span>
      )}
      <RowMenu label={`More for ${deploy.version}`} items={items} />
      {run.error && <ErrorMessage error={run.error} />}
      {removing && (
        <Dialog
          title="Remove this deploy?"
          eyebrow={`${deploy.version} · ${deploy.source.ref}`}
          onClose={() => {
            setRemoving(false);
          }}
          actions={
            <>
              <Button
                onClick={() => {
                  setRemoving(false);
                }}
              >
                Keep it
              </Button>
              <Button
                variant="danger"
                busy={busy('deploymentRemove')}
                busyLabel="Removing"
                onClick={() => {
                  run.mutate(
                    { name: 'deploymentRemove', args: { id: deploymentId } },
                    {
                      onSuccess: () => {
                        setRemoving(false);
                      },
                    },
                  );
                }}
              >
                Remove
              </Button>
            </>
          }
        >
          <p className={tabStyles.muted}>
            The provider deletes this deploy and its logs. What’s live now isn’t affected. This
            can’t be undone.
          </p>
          <ErrorMessage error={run.error} />
        </Dialog>
      )}
    </span>
  );
}

function DeploysTable({
  deploys,
  all,
  projectName,
  providerName,
}: {
  deploys: Deploy[];
  all: Deploy[];
  projectName: string;
  providerName: string;
}) {
  const [openSteps, setOpenSteps] = useState<string | null>(null);
  const [rollingTo, setRollingTo] = useState<Deploy | null>(null);
  const providerAction = useProviderAction();
  const command = useHoistCommand();
  // Some providers don't say how long a deploy took: no column of dashes then.
  const timed = all.some((deploy) => deploy.duration_ms !== null);
  const columns = timed ? 7 : 6;

  if (deploys.length === 0) {
    return <p className={styles.empty}>No deploys match these filters.</p>;
  }

  return (
    <>
      <table className={tabStyles.table}>
        <thead>
          <tr>
            <th scope="col" className={styles.versionCol}>
              Version
            </th>
            <th scope="col">Change</th>
            <th scope="col">By</th>
            {timed && <th scope="col">Duration</th>}
            <th scope="col">When</th>
            <th scope="col">Status</th>
            <th scope="col">
              <span className={tabStyles.visuallyHidden}>Actions</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {deploys.map((deploy) => (
            <Fragment key={deploy.id}>
              <tr
                className={styles.row}
                onClick={() => {
                  setOpenSteps(openSteps === deploy.id ? null : deploy.id);
                }}
                aria-expanded={openSteps === deploy.id}
              >
                <td className={cx(tabStyles.mono, tabStyles.strong, styles.versionCol)}>
                  <Version deploy={deploy} />
                </td>
                <td className={styles.changeCell}>
                  <span className={styles.changeRow}>
                    <span className={styles.change} title={deploy.source.message ?? undefined}>
                      {changeTitle(deploy)}
                    </span>
                    <MoreLines message={deploy.source.message} />
                  </span>
                  <DeploySource deploy={deploy} projectName={projectName} />
                </td>
                <td>
                  <span className={tabStyles.who}>
                    <ActorIcon actor={deploy.started_by} />
                    {shortName(deploy)}
                  </span>
                </td>
                {timed && (
                  <td className={tabStyles.muted}>
                    {deploy.duration_ms === null ? '—' : formatDuration(deploy.duration_ms)}
                  </td>
                )}
                <td className={cx(tabStyles.muted, tabStyles.nowrap)}>
                  {timeAgo(deploy.created_at)}
                </td>
                <td>
                  <DeployPill deploy={deploy} />
                </td>
                <td className={tabStyles.right}>
                  <DeployMenu
                    deploy={deploy}
                    providerAction={providerAction}
                    onRollBack={() => {
                      setRollingTo(deploy);
                    }}
                  />
                </td>
              </tr>
              {openSteps === deploy.id && (
                <tr className={styles.stepsRow}>
                  <td colSpan={columns}>
                    {deploy.failure && (
                      <p className={tabStyles.bad}>
                        {deploy.failure.message}
                        {deploy.started_by.kind === 'system' ? '' : ' The live site is unchanged.'}
                      </p>
                    )}
                    <DeployMore deploy={deploy} />
                    {deploy.steps.length > 0 ? (
                      <DeploySteps deploy={deploy} />
                    ) : (
                      <p className={tabStyles.muted}>
                        Built and released by {deploy.started_by.name}.
                        {deploy.url ? ` Served at ${deploy.url}.` : ''}
                      </p>
                    )}
                    <div className={styles.links}>
                      <Link className={styles.openDeploy} to={`/deploys/${deploy.id}`}>
                        Open deploy
                      </Link>
                      <DeployLinks deploy={deploy} providerName={providerName} />
                    </div>
                    <div className={styles.commands}>
                      <CopyField
                        text={command(['deploys', 'show', deploy.id, '--log'])}
                        label="command to show this deploy"
                      />
                      {canRollBackTo(deploy) && (
                        <CopyField
                          text={command(['rollback', deploy.id])}
                          label="command to roll back to this deploy"
                        />
                      )}
                    </div>
                  </td>
                </tr>
              )}
            </Fragment>
          ))}
        </tbody>
      </table>
      {providerAction.error && <ErrorMessage error={providerAction.error} />}
      {rollingTo && (
        <RollBackDialog
          projectName={projectName}
          providerName={isProviderDeploy(rollingTo.id) ? providerName : null}
          target={rollingTo}
          undone={changesUndoneBy(all, rollingTo)}
          onClose={() => {
            setRollingTo(null);
          }}
        />
      )}
    </>
  );
}

const PILLS: Record<DeployStatus, { label: string; dot: string | null }> = {
  queued: { label: 'Waiting', dot: null },
  running: { label: 'Deploying', dot: 'dotWarn' },
  live: { label: 'Earlier', dot: 'dotFaint' },
  failed: { label: 'Failed', dot: 'dotBad' },
  cancelled: { label: 'Stopped', dot: 'dotIdle' },
  rolled_back: { label: 'Earlier', dot: 'dotFaint' },
};

function DeployPill({ deploy }: { deploy: Deploy }) {
  if (deploy.current) {
    return (
      <span className={tabStyles.pill} title="Current production deployment">
        <span className={cx(tabStyles.dot, tabStyles.dotGood)} />
        Live
      </span>
    );
  }
  const { label, dot } = PILLS[deploy.status];
  return (
    <span className={tabStyles.pill}>
      <span className={cx(tabStyles.dot, dot && tabStyles[dot])} />
      {label}
    </span>
  );
}

export function RollBackDialog({
  projectName,
  providerName,
  target,
  undone,
  onClose,
}: {
  projectName: string;
  /** Set when the provider itself rolls back (a watched project). */
  providerName: string | null;
  target: Deploy;
  undone: Deploy[];
  onClose: () => void;
}) {
  const rollBack = useRollBack();

  return (
    <Dialog
      title={`Roll ${projectName} back to ${target.version}?`}
      eyebrow={changeTitle(target)}
      onClose={onClose}
      actions={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="accent"
            disabled={rollBack.isPending}
            onClick={() => {
              rollBack.mutate(target.id, { onSuccess: onClose });
            }}
          >
            Roll back to {target.version}
          </Button>
        </>
      }
    >
      <p className={styles.dialogText}>
        {providerName
          ? `${providerName} serves ${target.version} again in about a minute. Nothing is rebuilt.`
          : `${target.version} starts serving traffic again in about a minute. Nothing is rebuilt.`}
      </p>
      {undone.length > 0 && (
        <p className={styles.dialogText}>
          These changes leave the live site:{' '}
          {undone.map((deploy) => `${deploy.version} (${changeTitle(deploy)})`).join(', ')}. You can
          roll forward to them the same way.
        </p>
      )}
      {rollBack.error && <ErrorMessage error={rollBack.error} />}
    </Dialog>
  );
}

/**
 * Where a deploy came from, under its change: the branch and short commit for
 * a commit, the folder or upload otherwise. Nothing when that's only the
 * project's own name again.
 */
function DeploySource({ deploy, projectName }: { deploy: Deploy; projectName: string }) {
  const { kind, ref, commit } = deploy.source;
  const sha = commit?.slice(0, 7) ?? null;
  const isSha = sha !== null && commit?.startsWith(ref) === true;
  const branch = kind === 'commit' && !isSha && ref !== projectName ? ref : null;
  const where = kind === 'folder' && ref !== projectName ? ref : null;
  if (!branch && !sha && !where) return null;
  return (
    <span className={cx(tabStyles.sub, styles.source)}>
      {branch && (
        <span className={styles.branch}>
          <GitBranch size={12} aria-hidden="true" />
          {branch}
        </span>
      )}
      {sha && <span className={tabStyles.mono}>{sha}</span>}
      {where && <span>{where}</span>}
    </span>
  );
}

/** The rest of a deploy's commit message, below the subject the row shows. */
function DeployMore({ deploy }: { deploy: Deploy }) {
  const { body } = splitCommit(deploy.source.message);
  if (!body) return null;
  return <pre className={styles.commitBody}>{body}</pre>;
}

/** The version, as a link to its commit on GitHub when Hoista knows both. */
function Version({ deploy }: { deploy: Deploy }) {
  const url = commitUrl(deploy.source.repo, deploy.source.commit);
  if (!url) return <>{deploy.version}</>;
  return (
    <button
      type="button"
      className={styles.versionLink}
      title="Open the commit on GitHub"
      onClick={(event) => {
        event.stopPropagation();
        void openExternal(url);
      }}
    >
      {deploy.version}
    </button>
  );
}

/** Where else a deploy can be seen: its commit, and its page at the provider. */
export function DeployLinks({ deploy, providerName }: { deploy: Deploy; providerName: string }) {
  const commit = commitUrl(deploy.source.repo, deploy.source.commit);
  return (
    <>
      {commit && (
        <ExternalLink variant="text" url={commit} className={styles.openDeploy}>
          Commit on GitHub
        </ExternalLink>
      )}
      {deploy.provider_page && (
        <ExternalLink variant="text" url={deploy.provider_page} className={styles.openDeploy}>
          Open in {providerName}
        </ExternalLink>
      )}
    </>
  );
}
