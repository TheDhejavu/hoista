/**
 * Connect one service. Where the service offers it, Connect goes straight
 * to its own page to approve Hoista, with a token as the other way. The
 * rest get the token steps. Either way the engine's checks show once it is
 * saved. Opens over the Accounts screen at
 * `/accounts/connect/:serviceId`, and over the Projects screen from its provider tiles.
 */
import { providerGuide } from '@ui/lib/providerGuides';
import { createContext, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import type { ConnectGuide, ServiceId, TokenCheck } from '@shared/api';
import { isServiceId, entryOf, serviceName } from '@shared/providers';
import { useAccounts, useConnectGuide } from '@ui/api/accounts';
import { useFoundOnProviders } from '@ui/api/imports';
import { useRepositories } from '@ui/api/repositories';
import { ProjectPicker } from '@ui/screens/projects/ProjectPicker';
import { useApplyPicker, useFoundSnapshot, usePicker } from '@ui/screens/projects/usePicker';
import { Button } from '@ui/components/Button';
import { showToast } from '@ui/lib/toast';
import { timeAgo } from '@ui/lib/format';
import { ConnectLink } from '@ui/components/ConnectLink';
import { Dialog } from '@ui/components/Dialog';
import { SkeletonLines } from '@ui/components/Skeleton';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { ExternalLink } from '@ui/components/ExternalLink';
import { TokenField } from '@ui/components/TokenField';
import { QueryState } from '@ui/components/QueryState';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { Step, Steps } from '@ui/components/Steps';
import { TokenChecks } from '@ui/components/TokenChecks';
import { ProviderGuideDialog } from '@ui/screens/guides/ProviderGuideScreen';
import { TokenAccountFields, useBrowserSignIn, useTokenConnect } from './useConnect';
import styles from './ConnectDialog.module.css';

export function ConnectDialog() {
  const { serviceId } = useParams();
  const navigate = useNavigate();

  function close(): void {
    void navigate('/accounts');
  }

  if (!isServiceId(serviceId)) {
    return (
      <Dialog title="Can’t connect that yet" onClose={close}>
        <p>Hoista doesn’t have an adapter for that service yet.</p>
      </Dialog>
    );
  }
  return <ConnectServiceDialog service={serviceId} onClose={close} />;
}

/** The Connect dialog for one service, wherever it opens from. */
export function ConnectServiceDialog({
  service,
  onClose,
  stay = false,
}: {
  service: ServiceId;
  onClose: () => void;
  /**
   * Opened over another dialog (Settings): adding projects doesn't move to the
   * Projects screen, so the person comes back to what they were doing.
   */
  stay?: boolean;
}) {
  const guide = useConnectGuide(service);

  return (
    <StayContext value={stay}>
      <QueryState
        query={guide}
        skeleton={
          <Dialog
            title={`Connect ${serviceName(service)}`}
            icon={<ServiceIcon id={service} size={18} />}
            onClose={onClose}
          >
            <SkeletonLines count={4} />
          </Dialog>
        }
      >
        {(data) => <ConnectForm guide={data} onClose={onClose} />}
      </QueryState>
    </StayContext>
  );
}

/** Whether connecting stays over the dialog it opened from (see `stay`). */
const StayContext = createContext(false);

function ConnectForm({ guide, onClose }: { guide: ConnectGuide; onClose: () => void }) {
  const [checks, setChecks] = useState<TokenCheck[] | null>(null);
  const [withToken, setWithToken] = useState(guide.sign_in === null);
  // Step 2 after a token: the account is connected; pick what to add.
  const [picking, setPicking] = useState(false);
  // The provider's step-by-step guide, open over this dialog.
  const [guideStep, setGuideStep] = useState<'account' | 'token' | null>(null);
  const tokenConnect = useTokenConnect(guide, finish);

  /**
   * All checks passed: on to step 2, picking what to add. Otherwise show what
   * needs a look.
   */
  function finish(results: TokenCheck[]): void {
    if (results.every((check) => check.ok)) {
      setPicking(true);
      return;
    }
    setChecks(results);
  }

  const found = (useFoundOnProviders().data?.resources ?? []).filter(
    (resource) => resource.provider === guide.service,
  ).length;

  if (checks) {
    return (
      <Dialog
        title={`${guide.name} is connected, with gaps`}
        icon={<ServiceIcon id={guide.service} size={18} />}
        onClose={onClose}
        actions={
          <Button variant="accent" large onClick={onClose}>
            Done
          </Button>
        }
      >
        <div className={styles.body}>
          <ConnectLink service={guide.service} name={guide.name} state="connected" />
          <div className={styles.connectedRow}>
            <span>
              {found > 0
                ? `Hoista found ${String(found)} ${found === 1 ? 'project' : 'projects'} on ${guide.name}. Some checks below need a look.`
                : `Some checks below need a look before everything works.`}
            </span>
            <span className={styles.connectedPill}>Connected</span>
          </div>
          <TokenChecks checks={checks} />
        </div>
      </Dialog>
    );
  }

  if (picking) {
    return <PickWhatToAdd guide={guide} onClose={onClose} />;
  }

  if (guide.sign_in && !withToken) {
    return (
      <OnTheirPage
        guide={guide}
        onGaps={setChecks}
        onUseToken={() => {
          setWithToken(true);
        }}
        onClose={onClose}
      />
    );
  }

  return (
    <Dialog
      title={`Connect ${guide.name}`}
      icon={<ServiceIcon id={guide.service} size={18} />}
      onClose={onClose}
      actions={
        <>
          <Button large onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="accent"
            large
            disabled={!tokenConnect.canSubmit}
            onClick={tokenConnect.submit}
          >
            {tokenConnect.actionLabel}
          </Button>
        </>
      }
    >
      <div className={styles.body}>
        <ConnectLink
          service={guide.service}
          name={guide.name}
          state={tokenConnect.isBusy ? 'checking' : 'idle'}
        />
        <p>{guide.intro}</p>
        <TokenSteps guide={guide} onGuide={setGuideStep} />
        {guideStep && (
          <ProviderGuideDialog
            provider={guide.service}
            step={guideStep}
            onClose={() => {
              setGuideStep(null);
            }}
            onConnected={() => {
              setPicking(true);
            }}
          />
        )}
        <TokenField
          label={guide.token_label}
          value={tokenConnect.token}
          placeholder={guide.token_placeholder}
          onChange={tokenConnect.setToken}
        />
        <TokenAccountFields connect={tokenConnect} />
      </div>
    </Dialog>
  );
}

/**
 * Connect on the service's own page: Hoista opens it as soon as this shows,
 * and the dialog waits for you to click Allow there.
 */
function OnTheirPage({
  guide,
  onGaps,
  onUseToken,
  onClose,
}: {
  guide: ConnectGuide;
  /** Connected, but some checks need a look. */
  onGaps: (checks: TokenCheck[]) => void;
  onUseToken: () => void;
  onClose: () => void;
}) {
  // One dialog for both steps: it stays open and only its content changes.
  const [step, setStep] = useState<1 | 2>(1);
  const started = useRef(false);
  const pick = usePickStep(guide, onClose, step === 2);
  const signIn = useBrowserSignIn(guide, (checks) => {
    if (checks.every((check) => check.ok)) setStep(2);
    else onGaps(checks);
  });
  const done = signIn.done;
  const open = signIn.open;

  // Connect means "take me there", so the page opens straight away.
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    open();
  });

  if (step === 2) {
    return (
      <Dialog
        depth={1}
        title={pick.title}
        eyebrow={<StepMarks provider={guide.name} step={2} />}
        icon={<ServiceIcon id={guide.service} size={18} />}
        onClose={pick.notNow}
        wide
        sideAction={
          <Button
            large
            onClick={() => {
              setStep(1);
            }}
          >
            ← Back
          </Button>
        }
        actions={pick.actions}
      >
        {pick.body}
      </Dialog>
    );
  }

  return (
    <Dialog
      depth={0}
      title={`Allow Hoista on ${guide.name}`}
      eyebrow={<StepMarks provider={guide.name} step={1} />}
      icon={<ServiceIcon id={guide.service} size={18} />}
      onClose={onClose}
      actions={
        <>
          <Button large onClick={onClose}>
            Cancel
          </Button>
          {done ? (
            <Button
              variant="accent"
              large
              onClick={() => {
                setStep(2);
              }}
            >
              Next
            </Button>
          ) : (
            <Button variant="accent" large onClick={open}>
              {signIn.isPending ? `Open ${guide.name} again` : `Continue on ${guide.name}`}
            </Button>
          )}
        </>
      }
    >
      <div className={styles.body}>
        <ConnectLink
          service={guide.service}
          name={guide.name}
          state={done ? 'connected' : signIn.isPending ? 'checking' : 'idle'}
        />
        <p>{guide.intro}</p>
        <Steps>
          <Step number={1} title={`Hoista opens ${guide.name} in your browser`}>
            <span>Sign in there if it asks. No token to copy.</span>
          </Step>
          <Step number={2} title="Click Allow">
            <span>{guide.sign_in?.on_their_page}</span>
          </Step>
          <Step number={3} title="Come back here">
            <span>Hoista connects by itself and finds your projects.</span>
          </Step>
        </Steps>
        {done ? (
          <p className={styles.waitingLine} role="status">
            {guide.name} is connected.
          </p>
        ) : (
          signIn.isPending && (
            <p className={styles.waitingLine} role="status">
              Waiting for you to click Allow on {guide.name}’s page
            </p>
          )
        )}
        <ErrorMessage error={signIn.error} />
        {!done && (
          <div className={styles.alt}>
            <button type="button" className={styles.again} onClick={onUseToken}>
              Use a token instead
            </button>
            <span>Read-only checks first, then Hoista asks before it changes anything.</span>
          </div>
        )}
      </div>
    </Dialog>
  );
}

/** Get an account, make a token, paste it. */
function TokenSteps({
  guide,
  onGuide,
}: {
  guide: ConnectGuide;
  onGuide: (step: 'account' | 'token') => void;
}) {
  return (
    <Steps>
      <Step number={1} title={`Have a ${guide.name} account`}>
        <span className={styles.links}>
          {providerGuide(guide.service) ? (
            <button
              type="button"
              className={styles.linkButton}
              onClick={() => {
                onGuide('account');
              }}
            >
              I don’t have one. Walk me through it
            </button>
          ) : (
            <ExternalLink url={guide.signup_url}>I don’t have one</ExternalLink>
          )}
        </span>
      </Step>
      <Step number={2} title={`Create a token in ${guide.name}`}>
        <span>{guide.token_steps}</span>
        <span className={styles.links}>
          <ExternalLink url={guide.token_url}>Open the tokens page</ExternalLink>
          {providerGuide(guide.service) && (
            <button
              type="button"
              className={styles.linkButton}
              onClick={() => {
                onGuide('token');
              }}
            >
              Step-by-step guide
            </button>
          )}
        </span>
      </Step>
      <Step number={3} title="Paste it below" />
    </Steps>
  );
}

/** "Connect Railway ▬▭ Step 1 of 2": where you are in connecting, above the title. */
function StepMarks({ provider, step }: { provider: string; step: 1 | 2 }) {
  return (
    <span className={styles.steps}>
      Connect {provider}
      <span className={styles.marks} aria-hidden="true">
        <i className={styles.markOn} />
        <i className={step === 2 ? styles.markOn : styles.mark} />
      </span>
      Step {step} of 2
    </span>
  );
}

/**
 * Step 2: the account is connected. Pick the projects to add, each with
 * everything in it; they become watched projects. Nothing changes at the provider.
 */
function usePickStep(
  guide: ConnectGuide,
  onClose: () => void,
  /** Whether this step is showing. The sign-in flow sets it up at step 1. */
  active = true,
) {
  const navigate = useNavigate();
  const stay = useContext(StayContext);
  const found = useFoundSnapshot(guide.service, active);
  const snapshot = found.projects;
  const projects = useMemo(() => snapshot ?? [], [snapshot]);
  const picker = usePicker(projects, 'everything');
  const { apply, busy, error } = useApplyPicker();
  // What the scan of the newest account on this provider missed, if anything.
  const accounts = useAccounts();
  const newest = (accounts.data?.items ?? [])
    .filter((account) => account.service === guide.service)
    .sort((a, b) => b.connected_at.localeCompare(a.connected_at))[0];
  const scanNote = newest?.scan_note ?? null;
  // What was found before connecting is out of date the moment the account
  // is added: until the fresh read is back, it's still looking, never "nothing".
  const looking = snapshot === null && !found.error;
  // A code account (GitHub) runs nothing, so there's nothing to pick.
  const code = entryOf(guide.service).catalogue.category === 'code';
  // Nothing to pick: say why, plainly, instead of an empty picker.
  const empty = code || (!looking && !found.error && projects.length === 0);
  const inProjects = newest?.used_by ?? [];
  const created = picker.plan.create.length;
  const label = picker.workspaces.find((item) => item.id === picker.workspace)?.label;
  const other = picker.workspaces.find((item) => item.id !== picker.workspace);

  function notNow(): void {
    showToast(`${guide.name} is connected. Its projects are under Found on your providers.`);
    onClose();
  }

  async function add(): Promise<void> {
    if (await apply(picker.plan)) {
      showToast(
        created === 0
          ? 'Imported into your projects'
          : created === 1
            ? 'Imported 1 new project'
            : `Imported ${String(created)} new projects`,
      );
      onClose();
      if (!stay) void navigate('/');
    }
  }

  function done(): void {
    showToast(`${guide.name} is connected`);
    onClose();
  }

  const actions = empty ? (
    <Button variant="accent" large onClick={done}>
      Done
    </Button>
  ) : (
    <>
      <span className={styles.pickSummary}>
        {picker.summary || (label ? `Nothing chosen from ${label}` : '')}
        {other ? ` · also ${other.label} (switch Workspace to see it)` : ''}
      </span>
      <Button large onClick={notNow}>
        Skip for now
      </Button>
      <Button
        variant="accent"
        large
        busy={busy}
        busyLabel="Importing"
        disabled={!picker.changed}
        onClick={() => {
          void add();
        }}
      >
        Import
      </Button>
    </>
  );

  const body = code ? (
    <CodeConnected service={guide.service} name={guide.name} accountId={newest?.id ?? null} />
  ) : empty ? (
    <div className={styles.nothing} role="status">
      <span className={styles.nothingIcon}>
        <ServiceIcon id={guide.service} size={22} />
      </span>
      {scanNote ? (
        <>
          <h3>Hoista couldn’t see everything on {guide.name}</h3>
          <p>{scanNote}</p>
        </>
      ) : inProjects.length > 0 ? (
        <>
          <h3>Everything on {guide.name} is already in Hoista</h3>
          <p>
            It’s in {inProjects.join(', ')}. New things you or your agent make on {guide.name} show
            up under Found on your providers.
          </p>
        </>
      ) : (
        <>
          <h3>Nothing on {guide.name} yet</h3>
          <p>
            Hoista didn’t find anything on this account. When you or your agent make something on{' '}
            {guide.name}, it shows up under Found on your providers.
          </p>
        </>
      )}
    </div>
  ) : (
    <div className={styles.body}>
      <p>
        {guide.name} is connected.{' '}
        {entryOf(guide.service).groupWord
          ? `Each ${guide.name} ${entryOf(guide.service).groupWord ?? 'project'} is imported into a Hoista project of the same name.`
          : `${guide.name} has no projects, so each of its ${(entryOf(guide.service).projectsWord ?? 'resources').toLowerCase()} becomes a Hoista project of its own name.`}{' '}
        Change where it goes, untick what to leave out, or drag a resource to another project.
        Nothing changes on {guide.name}.
      </p>
      {looking ? (
        <p role="status">Looking for your projects on {guide.name}</p>
      ) : (
        <>
          {scanNote && <p role="note">{scanNote}</p>}
          <ProjectPicker picker={picker} provider={guide.service} />
        </>
      )}
      <ErrorMessage error={error ?? found.error} />
    </div>
  );

  const title = empty ? `${guide.name} is connected` : 'Import projects';
  return { body, actions, notNow: empty ? done : notNow, title };
}

/**
 * A code account is connected: the repositories it reaches, and where they
 * get used. Projects link to one in Settings; agents find them with
 * `list_repositories`.
 */
function CodeConnected({
  service,
  name,
  accountId,
}: {
  service: ServiceId;
  name: string;
  accountId: string | null;
}) {
  const repos = useRepositories(accountId);
  const list = repos.data;
  return (
    <div className={styles.body}>
      <p>
        {name} is connected. Link a project to one of these in the project’s Settings, under Code.
        Your agents can find them too and push there; providers that build from {name} deploy the
        push themselves.
      </p>
      {list === undefined && !repos.error ? (
        <p role="status" className={styles.repoLoading}>
          <span className={styles.repoSpinner} aria-hidden="true" />
          Reading your repositories on {name}
        </p>
      ) : list?.length === 0 ? (
        <div className={styles.nothing} role="status">
          <span className={styles.nothingIcon}>
            <ServiceIcon id={service} size={22} />
          </span>
          <h3>This token reaches no repositories</h3>
          <p>
            Give the token access to the repositories your projects are built from, then reconnect.
          </p>
        </div>
      ) : list ? (
        <div className={styles.repoFrame}>
          <table className={styles.repoTable}>
            <caption className="visually-hidden">Repositories on {name}</caption>
            <thead>
              <tr>
                <th scope="col">
                  {list.length} {list.length === 1 ? 'repository' : 'repositories'}
                </th>
                <th scope="col">Visibility</th>
                <th scope="col">Last push</th>
              </tr>
            </thead>
            <tbody>
              {list.map((repo) => (
                <tr key={repo.full_name}>
                  <td className="mono">{repo.full_name}</td>
                  <td>{repo.private ? 'Private' : 'Public'}</td>
                  <td>{repo.pushed_at ? timeAgo(repo.pushed_at) : '—'}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : null}
      <ErrorMessage error={repos.error} />
    </div>
  );
}

/** Step 2 after connecting with a token. */
function PickWhatToAdd({ guide, onClose }: { guide: ConnectGuide; onClose: () => void }) {
  const pick = usePickStep(guide, onClose);
  return (
    <Dialog
      title={pick.title}
      eyebrow={<StepMarks provider={guide.name} step={2} />}
      icon={<ServiceIcon id={guide.service} size={18} />}
      onClose={pick.notNow}
      wide
      actions={pick.actions}
    >
      {pick.body}
    </Dialog>
  );
}
