/**
 * Connecting a project's source, in three steps: choose it, review what
 * changes, then do it. A repository the provider doesn't deploy yet is
 * connected at the provider and deployed, so what Hoista shows is what runs;
 * the deploy's own page then follows the build and its log. A folder, or
 * the repository the provider already deploys, changes nothing at the provider.
 */
import { AlertTriangle, Check, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import type { ProjectDetail, Deploy, Resource } from '@shared/api';
import { providerEntry, serviceName } from '@shared/providers';
import { useProviderDetails, useLinkSource, usePreviewSource } from '@ui/api/projects';
import { PillSelect } from '@ui/components/Select';
import { useProviderAction, useProviderChange, fetchDeploys } from '@ui/api/deploys';
import { useLinkRepository } from '@ui/api/repositories';
import { openExternal } from '@ui/api/system';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { cx } from '@ui/lib/cx';
import { SourcePicker, type SourceChoice } from './SourcePicker';
import styles from './SourcePicker.module.css';
import { MINUTE_MS, SECOND_MS } from '@shared/time';
import { isProviderDeploy } from '@shared/deployIds';

/** How long to wait for the deploy a provider starts by itself when a repository is connected. */
const OWN_DEPLOY_WAIT_MS = 8 * SECOND_MS;
/** How long to wait for the deploy Hoista asked for to show up. */
const DEPLOY_WAIT_MS = MINUTE_MS;
const DEPLOY_POLL_MS = 2.5 * SECOND_MS;

type Step = 'choose' | 'review' | 'run';
type RunStep = 'provider' | 'link' | 'deploy' | 'find';
type RunState = 'idle' | 'now' | 'done' | 'failed';

/** What connecting the choice does. */
type Plan =
  | { kind: 'switch' } // a new repository: connect it at the provider, then deploy
  | { kind: 'record' } // the one the provider deploys, or a folder: Hoista only
  | { kind: 'blocked'; why: string };

export function ChangeSourceDialog({
  detail,
  services,
  initial,
  onClose,
}: {
  detail: ProjectDetail;
  /** The project's resources that run code (a frontend, an API, a worker). */
  services: Resource[];
  /** The service it was opened for, when one was. */
  initial: Resource | null;
  onClose: () => void;
}) {
  const { project } = detail;
  // One service needs no choosing; several start at None.
  const [serviceId, setServiceId] = useState<string | null>(
    initial?.id ?? (services.length === 1 ? (services[0]?.id ?? null) : null),
  );
  const codeResource = services.find((service) => service.id === serviceId) ?? null;
  const latest = useProviderDetails(codeResource?.id ?? null).data?.deployments.find(
    (deployment) => deployment.repo && deployment.commit,
  );
  const deployed =
    latest?.repo && latest.commit ? { repo: latest.repo, commit: latest.commit } : null;
  const navigate = useNavigate();
  const provider = codeResource ? serviceName(codeResource.provider) : 'the provider';
  const canConnect = codeResource
    ? providerEntry(codeResource.provider)?.connectRepo === true
    : false;
  const [step, setStep] = useState<Step>('choose');
  const [choice, setChoice] = useState<SourceChoice | null>(null);
  const preview = usePreviewSource(project.id);
  const providerChange = useProviderChange();
  const providerAction = useProviderAction();
  const linkRepository = useLinkRepository(project.id);
  const linkSource = useLinkSource(project.id);
  const [states, setStates] = useState<Record<RunStep, RunState>>({
    provider: 'idle',
    link: 'idle',
    deploy: 'idle',
    find: 'idle',
  });
  const [failure, setFailure] = useState<Error | null>(null);
  const [notFound, setNotFound] = useState(false);
  const running = Object.values(states).includes('now');

  const plan: Plan | null = !choice
    ? null
    : choice.kind === 'folder' || choice.repository === deployed?.repo
      ? { kind: 'record' }
      : !codeResource
        ? {
            kind: 'blocked',
            why: 'Nothing in this project runs code at a provider, so there’s nothing to deploy.',
          }
        : canConnect
          ? { kind: 'switch' }
          : {
              kind: 'blocked',
              why: `${provider} can’t change the repository it deploys through Hoista. Change it in ${provider}’s project settings first; then it shows here as the suggested repository.`,
            };

  // A folder is read when it's chosen, so the review shows what's in it.
  const previewMutate = preview.mutate;
  useEffect(() => {
    if (choice?.kind === 'folder') previewMutate(choice.path);
  }, [choice, previewMutate]);

  const mark = (name: RunStep, state: RunState) => {
    setStates((now) => ({ ...now, [name]: state }));
  };

  const providerDeploys = () => listProviderDeploys(project.id, codeResource?.id ?? '');
  const newDeploy = (before: Set<string>, ms: number) =>
    waitForNewDeploy(project.id, codeResource?.id ?? '', before, ms);

  async function record(): Promise<void> {
    if (!choice) return;
    setStep('run');
    setFailure(null);
    mark('link', 'now');
    try {
      if (choice.kind === 'folder') {
        await linkSource.mutateAsync({ kind: 'folder', path: choice.path });
      } else if (choice.commit) {
        await linkSource.mutateAsync({
          kind: 'github',
          repository: choice.repository,
          commit: choice.commit,
        });
      } else if (choice.accountId) {
        await linkRepository.mutateAsync({
          accountId: choice.accountId,
          repository: choice.repository,
          branch: choice.branch,
        });
      }
      mark('link', 'done');
      onClose();
    } catch (error) {
      mark('link', 'failed');
      setFailure(error instanceof Error ? error : new Error(String(error)));
    }
  }

  async function connectAndDeploy(): Promise<void> {
    if (choice?.kind !== 'github' || !codeResource) return;
    setStep('run');
    setFailure(null);
    setNotFound(false);
    let now: RunStep = 'provider';
    try {
      const before = new Set((await providerDeploys()).map((deploy) => deploy.id));
      mark('provider', 'now');
      await providerChange.mutateAsync({
        resourceId: codeResource.id,
        body: {
          action: 'connect_repo',
          repo: choice.repository,
          ...(choice.branch ? { branch: choice.branch } : {}),
        },
      });
      mark('provider', 'done');
      now = 'link';
      mark('link', 'now');
      if (choice.accountId) {
        await linkRepository.mutateAsync({
          accountId: choice.accountId,
          repository: choice.repository,
          branch: choice.branch,
        });
      }
      mark('link', 'done');
      now = 'deploy';
      mark('deploy', 'now');
      // Connecting a repository often starts a deploy by itself; start one
      // only when the provider didn't.
      let started = await newDeploy(before, OWN_DEPLOY_WAIT_MS);
      if (!started) {
        await providerAction.mutateAsync({ resourceId: codeResource.id, action: 'deploy_latest' });
      }
      mark('deploy', 'done');
      now = 'find';
      mark('find', 'now');
      started ??= await newDeploy(before, DEPLOY_WAIT_MS);
      if (!started) {
        mark('find', 'done');
        setNotFound(true);
        return;
      }
      mark('find', 'done');
      onClose();
      void navigate(`/deploys/${encodeURIComponent(started.id)}`);
    } catch (error) {
      mark(now, 'failed');
      setFailure(error instanceof Error ? error : new Error(String(error)));
    }
  }

  const deploys = plan?.kind === 'switch';
  const stepNames = ['Choose', 'Review', deploys ? 'Deploy' : 'Connect'];
  const stepIndex = step === 'choose' ? 0 : step === 'review' ? 1 : 2;

  return (
    <Dialog
      title={project.source.kind === 'unlinked' ? 'Connect a source' : 'Change source'}
      eyebrow={project.name}
      wide
      onClose={() => {
        // A run finishes before the window goes.
        if (!running) onClose();
      }}
      actions={
        step === 'choose' ? (
          <>
            <Button variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button
              variant="accent"
              disabled={!choice || (services.length > 0 && !codeResource)}
              onClick={() => {
                setStep('review');
              }}
            >
              Review
            </Button>
          </>
        ) : step === 'review' ? (
          <>
            <Button
              variant="ghost"
              onClick={() => {
                setStep('choose');
              }}
            >
              Back
            </Button>
            {plan?.kind === 'blocked' &&
              codeResource &&
              providerEntry(codeResource.provider)?.dashboard && (
                <Button
                  onClick={() => {
                    void openExternal(providerEntry(codeResource.provider)?.dashboard?.('') ?? '');
                  }}
                >
                  Open {provider} ↗
                </Button>
              )}
            <Button
              variant="accent"
              disabled={
                !plan || plan.kind === 'blocked' || (choice?.kind === 'folder' && preview.isPending)
              }
              onClick={() => {
                void (deploys ? connectAndDeploy() : record());
              }}
            >
              {deploys
                ? 'Connect and deploy'
                : choice?.kind === 'folder'
                  ? 'Connect folder'
                  : 'Connect'}
            </Button>
          </>
        ) : failure || notFound ? (
          <>
            <Button variant="ghost" onClick={onClose}>
              Close
            </Button>
            {notFound ? (
              <Button
                variant="accent"
                onClick={() => {
                  onClose();
                  void navigate(`/projects/${project.id}/deploys`);
                }}
              >
                Open Deploys
              </Button>
            ) : (
              <Button
                variant="accent"
                onClick={() => {
                  void (deploys ? connectAndDeploy() : record());
                }}
              >
                Try again
              </Button>
            )}
          </>
        ) : undefined
      }
    >
      <ol className={styles.steps} aria-label="Steps">
        {stepNames.map((name, index) => (
          <li
            key={name}
            className={cx(
              index === stepIndex && styles.stepNow,
              index < stepIndex && styles.stepDone,
            )}
            aria-current={index === stepIndex ? 'step' : undefined}
          >
            <span className={styles.stepNumber}>
              {index < stepIndex ? <Check size={12} aria-hidden="true" /> : index + 1}
            </span>
            {name}
          </li>
        ))}
      </ol>

      {step === 'choose' && services.length > 1 && (
        <div className={styles.serviceRow}>
          <span className={styles.branchLabel}>Which service builds from it?</span>
          <PillSelect
            label="Service"
            value={serviceId}
            options={[
              { value: null as string | null, label: 'None' },
              ...services.map((service) => ({
                value: service.id,
                label: service.name,
                note: serviceName(service.provider),
              })),
            ]}
            onChange={(id) => {
              setServiceId(id);
              setChoice(null);
            }}
          />
        </div>
      )}
      {step === 'choose' && (
        <SourcePicker
          provider={provider}
          offer={
            deployed && deployed.repo !== project.source.ref.split('@')[0]
              ? { repository: deployed.repo, commit: deployed.commit }
              : null
          }
          choice={choice}
          onChoose={setChoice}
        />
      )}

      {step === 'review' && choice && plan && (
        <div className={styles.review}>
          {choice.kind === 'github' ? (
            <RepositoryReview
              choice={choice}
              plan={plan}
              provider={provider}
              resource={codeResource}
              deployed={deployed}
            />
          ) : (
            <FolderReview
              provider={provider}
              deployed={deployed}
              preview={preview.data ?? null}
              loading={preview.isPending}
              error={preview.error}
            />
          )}
        </div>
      )}

      {step === 'run' && (
        <div className={styles.review}>
          <ol className={styles.run}>
            {(deploys
              ? ([
                  [
                    'provider',
                    `Connect ${choice?.kind === 'github' ? choice.repository : ''} at ${provider}`,
                  ],
                  ['link', 'Record it in Hoista'],
                  ['deploy', 'Start a deploy of the newest commit'],
                  ['find', 'Open the deploy to follow its build'],
                ] as const)
              : ([['link', 'Record it in Hoista']] as const)
            ).map(([name, label]) => (
              <li
                key={name}
                className={cx(
                  states[name] === 'now' && styles.runNow,
                  states[name] === 'done' && styles.runDone,
                  states[name] === 'failed' && styles.runFailed,
                )}
              >
                <span className={styles.runIcon}>
                  {states[name] === 'now' ? (
                    <span className={styles.spinner} aria-hidden="true" />
                  ) : states[name] === 'done' ? (
                    <Check size={15} aria-hidden="true" />
                  ) : states[name] === 'failed' ? (
                    <X size={15} aria-hidden="true" />
                  ) : (
                    <span className={styles.dotIdle} aria-hidden="true" />
                  )}
                </span>
                {label}
              </li>
            ))}
          </ol>
          {notFound && (
            <p className={styles.notice}>
              The deploy started, but {provider} hasn’t listed it yet. It shows in Deploys in a
              moment.
            </p>
          )}
          <ErrorMessage error={failure} />
        </div>
      )}
    </Dialog>
  );
}

/** The provider deploys of one resource that Hoista lists now. */
async function listProviderDeploys(projectId: string, resourceId: string): Promise<Deploy[]> {
  const page = await fetchDeploys(projectId);
  return page.items.filter(
    (deploy) => isProviderDeploy(deploy.id) && deploy.resource_id === resourceId,
  );
}

/** Waits for a deploy of the resource that wasn't there before, up to `ms`. */
async function waitForNewDeploy(
  projectId: string,
  resourceId: string,
  before: Set<string>,
  ms: number,
): Promise<Deploy | null> {
  const until = Date.now() + ms;
  while (Date.now() < until) {
    const found = (await listProviderDeploys(projectId, resourceId)).find(
      (deploy) => !before.has(deploy.id),
    );
    if (found) return found;
    await new Promise((resolve) => setTimeout(resolve, DEPLOY_POLL_MS));
  }
  return null;
}

function RepositoryReview({
  choice,
  plan,
  provider,
  resource,
  deployed,
}: {
  choice: Extract<SourceChoice, { kind: 'github' }>;
  plan: Plan;
  provider: string;
  resource: Resource | null;
  deployed: { repo: string; commit: string } | null;
}) {
  const branch = choice.branch || 'its default branch';
  return (
    <>
      <p className={styles.reviewLabel}>What changes</p>
      <div className={styles.frame}>
        <table className={styles.compare}>
          <thead>
            <tr>
              <th scope="col" />
              <th scope="col">Now</th>
              <th scope="col">After</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <th scope="row">Repository</th>
              <td className="mono">{deployed?.repo ?? 'Not reported'}</td>
              <td className="mono">{choice.repository}</td>
            </tr>
            {choice.branch && (
              <tr>
                <th scope="row">Branch</th>
                <td>—</td>
                <td className="mono">{choice.branch}</td>
              </tr>
            )}
            <tr>
              <th scope="row">Live version</th>
              <td className="mono">{deployed ? `Commit ${deployed.commit.slice(0, 7)}` : '—'}</td>
              <td>
                {plan.kind === 'switch'
                  ? `The newest commit on ${branch}, once it’s built`
                  : 'Unchanged'}
              </td>
            </tr>
            {resource && (
              <tr>
                <th scope="row">Builds</th>
                <td colSpan={2}>
                  {resource.name} on {provider}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      {plan.kind === 'switch' && (
        <>
          <p className={styles.reviewLabel}>What happens</p>
          <ol className={styles.happens}>
            <li>
              {provider} builds {resource?.name} from {choice.repository} from now on.
            </li>
            <li>
              Hoista starts a deploy of the newest commit on {branch}. What’s live keeps serving
              until the new version is.
            </li>
            <li>You follow the build and its log on the deploy’s page.</li>
            <li>If it’s wrong, roll back to what’s live now from Deploys.</li>
          </ol>
        </>
      )}
      {plan.kind === 'record' && (
        <p className={styles.notice}>
          {provider} already deploys this repository. Hoista and your agents use it; nothing changes
          at {provider} and nothing deploys.
        </p>
      )}
      {plan.kind === 'blocked' && (
        <p className={styles.notice}>
          <AlertTriangle size={16} aria-hidden="true" />
          <span>{plan.why}</span>
        </p>
      )}
    </>
  );
}

function FolderReview({
  provider,
  deployed,
  preview,
  loading,
  error,
}: {
  provider: string;
  deployed: { repo: string; commit: string } | null;
  preview: import('@shared/api').SourcePreview | null;
  loading: boolean;
  error: Error | null;
}) {
  if (loading) {
    return (
      <p className={styles.status} role="status">
        <span className={styles.spinner} aria-hidden="true" />
        Reading the folder
      </p>
    );
  }
  if (error || !preview) return <ErrorMessage error={error} />;
  const rows: [string, string][] = [
    ['Folder', preview.path],
    ...(preview.framework ? [['Framework', preview.framework] as [string, string]] : []),
    ...(preview.package_manager
      ? [['Package manager', preview.package_manager] as [string, string]]
      : []),
    ...(preview.build_command.length > 0
      ? [['Build command', preview.build_command.join(' ')] as [string, string]]
      : []),
    ...(preview.env_names.length > 0
      ? [['Variables it reads', preview.env_names.join(', ')] as [string, string]]
      : []),
  ];
  return (
    <>
      <p className={styles.reviewLabel}>What’s in it</p>
      <div className={styles.frame}>
        <table className={styles.compare}>
          <tbody>
            {rows.map(([label, value]) => (
              <tr key={label}>
                <th scope="row">{label}</th>
                <td
                  className={label === 'Folder' || label === 'Build command' ? 'mono' : undefined}
                >
                  {value}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {preview.unrecognised && (
        <p className={styles.notice}>
          <AlertTriangle size={16} aria-hidden="true" />
          <span>
            Hoista didn’t recognise the project here. You can still connect it as the folder you
            work in. Check it’s the project’s root.
          </span>
        </p>
      )}
      {preview.warnings.map((warning) => (
        <p key={warning} className={styles.notice}>
          {warning}
        </p>
      ))}
      <p className={styles.notice}>
        <span>
          <strong>Nothing deploys.</strong> {provider} keeps building from{' '}
          {deployed ? deployed.repo : 'what it deploys now'}. You and your agents work on the code
          in this folder.
        </span>
      </p>
    </>
  );
}
