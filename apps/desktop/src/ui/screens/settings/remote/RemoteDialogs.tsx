/**
 * The three remote flows, each in its own dialog:
 *
 * - Set up: pick one of your accounts on a provider Hoista can run on; Hoista
 *   starts its published image there (after Touch ID), waits for it to
 *   answer, then opens its claim page with the one-time code. Or pick your
 *   own server: give the address of a Hoista you run yourself, with the
 *   self-hosting guide a click away.
 * (Connecting one you already have is Add a remote, in AddRemoteDialog.)
 * - Remove: stop showing the server here (it keeps running at the provider).
 * - Stop everything: cut every agent off and delete what the cloud copy holds,
 *   for when something looks wrong. Projects keep running.
 */
import { useMemo, useState } from 'react';
import { Check, Loader2, Server } from 'lucide-react';
import type { ProviderId, RemoteStatus, RemoteTarget, ServerSetup } from '@shared/api';
import { serviceName } from '@shared/providers';
import { useAccounts } from '@ui/api/accounts';
import { useAgents } from '@ui/api/agents';
import {
  useConnectServer,
  useForgetServer,
  useServerProviders,
  useServerReady,
  useSetUpServer,
  useSettings,
  useStopEverything,
} from '@ui/api/settings';
import { openExternal } from '@ui/api/system';
import { Button } from '@ui/components/Button';
import { CopyField } from '@ui/components/CopyField';
import { Dialog, DialogText } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { TextField } from '@ui/components/Field';
import { SelectField, type Option } from '@ui/components/Select';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { remotePlace } from '@ui/lib/remoteProviders';
import { cx } from '@ui/lib/cx';
import { plural } from '@ui/lib/format';
import { ConnectServiceDialog } from '@ui/screens/accounts/ConnectDialog';
import styles from './RemoteDialogs.module.css';
import { LOCAL_REMOTE_ADDRESS } from '@shared/ports';

function ProgressList({ steps, done }: { steps: string[]; done: number }) {
  return (
    <ol className={styles.progress} aria-live="polite">
      {steps.map((step, index) => {
        const state = index < done ? 'done' : index === done ? 'doing' : 'todo';
        return (
          <li key={step} className={cx(styles.progressItem, styles[state])}>
            <span className={styles.progressIcon} aria-hidden="true">
              {state === 'done' ? (
                <Check size={13} />
              ) : state === 'doing' ? (
                <Loader2 size={13} className={styles.spin} />
              ) : null}
            </span>
            {step}
          </li>
        );
      })}
    </ol>
  );
}

/**
 * Set up: pick one of your accounts on any provider Hoista can run on. Hoista
 * starts its published image there (pinned to the exact version), waits for
 * it to answer, then opens its claim page.
 */
/**
 * Where a remote runs: a connected account, a provider to connect first, or
 * a server of yours already running Hoista.
 */
type Where = { account: string } | { provider: ProviderId } | { own: true };

function sameWhere(a: Where | null, b: Where | null): boolean {
  if (!a || !b) return a === b;
  if ('account' in a && 'account' in b) return a.account === b.account;
  if ('provider' in a && 'provider' in b) return a.provider === b.provider;
  return 'own' in a && 'own' in b;
}

/** How to run Hoista on a server of your own, start to finish. */
const SELF_HOST_GUIDE = 'https://docs.hoista.dev/guides/self-host';

export function SetUpRemoteDialog({
  initialTarget,
  onClose,
  onConnectAgent,
  onBack,
  backLabel,
  depth,
}: {
  /** Its place in a multi-step flow, for the slide between steps. */
  depth?: number;
  /** A step of another flow (Add a remote): back returns there, named so. */
  onBack?: () => void;
  backLabel?: string;
  /** The provider picked in the settings row, if any. */
  initialTarget?: RemoteTarget | null;
  onClose: () => void;
  onConnectAgent: () => void;
}) {
  const providers = useServerProviders();
  const [connecting, setConnecting] = useState<ProviderId | null>(null);
  const accounts = useAccounts();
  const offered = useMemo(() => providers.data?.providers ?? [], [providers.data]);
  const usable = useMemo(
    () =>
      (accounts.data?.items ?? []).filter((item) =>
        offered.some((provider) => provider === item.service),
      ),
    [accounts.data, offered],
  );
  // Where it runs: one of your accounts, or a provider to connect first.
  // Nothing is picked until you choose, unless Settings already named one.
  const [choice, setChoice] = useState<Where | null>(null);
  // Accounts there before connecting one here, so the new one is picked once it's in.
  const [before, setBefore] = useState<string[] | null>(null);
  const added = before ? usable.find((item) => !before.includes(item.id)) : undefined;
  const picked: Where | null =
    (added ? { account: added.id } : choice) ??
    (() => {
      if (initialTarget === 'server') return { own: true };
      const named = usable.find((item) => item.service === initialTarget);
      return named ? { account: named.id } : null;
    })();
  const own = picked !== null && 'own' in picked;
  const account =
    picked && 'account' in picked ? usable.find((item) => item.id === picked.account) : undefined;
  const toConnect = picked && 'provider' in picked ? picked.provider : null;

  const whereChoices = useMemo<Option<Where | null>[]>(
    () => [
      { value: null, label: 'None' },
      ...usable.map((item) => ({
        value: { account: item.id },
        label: `${serviceName(item.service)} · ${item.label}`,
        group: 'Your accounts',
      })),
      ...offered.map((provider) => ({
        value: { provider },
        label: `Connect ${serviceName(provider)}`,
        group: usable.length > 0 ? 'Another account' : 'Connect a provider',
      })),
      { value: { own: true }, label: 'Your own server', group: 'Run it yourself' },
    ],
    [usable, offered],
  );
  const setUp = useSetUpServer();
  // Your own server: its address, checked that it answers.
  const [address, setAddress] = useState('');
  const [guideOpened, setGuideOpened] = useState(false);
  const connectServer = useConnectServer();
  const remoteUrl = useSettings().data?.agents.remote.url ?? null;
  const [made, setMade] = useState<ServerSetup | null>(null);
  const ready = useServerReady(made !== null);
  const [claimed, setClaimed] = useState(false);
  const [sealOpened, setSealOpened] = useState(false);
  const isReady = made !== null && ready.data?.ready === true;
  const place = account ? serviceName(account.service) : 'your cloud';
  const steps = [
    'Confirm it’s you',
    `Create Hoista in your ${place} account`,
    'Start it (a few minutes the first time)',
    'Claim it with a passkey',
  ];
  const done = isReady ? 3 : made ? 2 : setUp.isPending ? 0 : -1;
  const icon = account?.service ?? toConnect ?? '';

  // Connecting a provider is a step of this same window: it replaces this
  // one, and closing it comes back here with the new account picked.
  if (connecting) {
    return (
      <ConnectServiceDialog
        service={connecting}
        stay
        onClose={() => {
          setConnecting(null);
        }}
      />
    );
  }

  if (connectServer.isSuccess) {
    const url = remoteUrl ?? address.trim();
    return (
      <Dialog
        {...(depth === undefined ? {} : { depth: depth + 1 })}
        title="Your server is your remote"
        eyebrow="Instances"
        icon={<Server size={18} aria-hidden="true" />}
        onClose={onClose}
        actions={
          <>
            <Button variant="ghost" onClick={onClose}>
              Close
            </Button>
            <Button variant="primary" onClick={onConnectAgent}>
              Connect a chat app
            </Button>
          </>
        }
      >
        <div className={styles.body}>
          <div className={styles.ready}>
            <span className={styles.label}>Agents connect at</span>
            <CopyField text={`${url.replace(/\/$/, '')}/mcp`} label="your Hoista’s address" />
            <span className={styles.hint}>
              The first time an agent connects, you allow it on your server’s page with your
              passkey.
            </span>
          </div>
        </div>
      </Dialog>
    );
  }

  if (made) {
    return (
      <Dialog
        {...(depth === undefined ? {} : { depth: depth + 1 })}
        title={isReady ? 'Your Hoista is ready' : 'Starting your Hoista'}
        eyebrow="Instances"
        icon={<ServiceIcon id={icon} size={18} />}
        onClose={onClose}
        actions={
          <>
            <Button variant="ghost" onClick={claimed ? onConnectAgent : onClose}>
              {claimed ? 'Connect a chat app' : 'Close'}
            </Button>
            <Button
              variant="primary"
              disabled={!isReady}
              onClick={() => {
                void openExternal(made.claim_link).then(() => {
                  setClaimed(true);
                });
              }}
            >
              {!isReady ? 'Waiting for it to start' : claimed ? 'Open again' : 'Claim it'}
            </Button>
          </>
        }
      >
        <div className={styles.body}>
          <ProgressList steps={steps} done={done} />
          <div className={styles.ready}>
            <span className={styles.label}>Agents connect at</span>
            <CopyField text={`${made.url}/mcp`} label="your Hoista’s address" />
            <span className={styles.hint}>
              {claimed
                ? 'Opened in your browser. Make a passkey there; the link then stops working.'
                : 'Claiming opens your Hoista in the browser with its one-time code filled in. Only you have that link.'}
            </span>
          </div>
          {!made.key_hidden && made.seal_help ? (
            <div className={styles.seal}>
              <span className={styles.label}>One more step at {place}</span>
              <span className={styles.hint}>{made.seal_help}</span>
              {made.seal_page ? (
                <div>
                  <Button
                    onClick={() => {
                      void openExternal(made.seal_page ?? '').then(() => {
                        setSealOpened(true);
                      });
                    }}
                  >
                    {sealOpened ? `Opened ${place} in your browser` : `Open ${place}`}
                  </Button>
                </div>
              ) : null}
            </div>
          ) : null}
        </div>
      </Dialog>
    );
  }

  const working = setUp.isPending || connectServer.isPending;
  return (
    <Dialog
      {...(depth === undefined ? {} : { depth })}
      title={own ? 'Use Hoista on your own server' : 'Keep Hoista running in your cloud'}
      eyebrow="Instances"
      icon={
        icon && !own ? <ServiceIcon id={icon} size={18} /> : <Server size={18} aria-hidden="true" />
      }
      onClose={working ? () => undefined : onClose}
      {...(onBack && !working ? { onBack, backLabel: backLabel ?? 'Back' } : {})}
      actions={
        <>
          <Button variant="ghost" disabled={working} onClick={onClose}>
            Cancel
          </Button>
          {own ? (
            <Button
              variant="primary"
              disabled={!address.trim()}
              busy={connectServer.isPending}
              busyLabel="Checking it answers"
              onClick={() => {
                connectServer.mutate(address.trim());
              }}
            >
              Connect
            </Button>
          ) : account || !toConnect ? (
            <Button
              disabled={working || !account}
              variant="primary"
              busy={working}
              onClick={() => {
                if (!account) return;
                setUp.mutate(
                  { account_id: account.id },
                  {
                    onSuccess: (setup) => {
                      setMade(setup);
                    },
                  },
                );
              }}
            >
              {working ? 'Confirm it’s you' : account ? `Set up on ${place}` : 'Set up'}
            </Button>
          ) : (
            <Button
              variant="primary"
              onClick={() => {
                setBefore(usable.map((item) => item.id));
                setConnecting(toConnect);
              }}
            >
              {`Connect ${serviceName(toConnect)}`}
            </Button>
          )}
        </>
      }
    >
      <div className={styles.body}>
        {setUp.isPending ? (
          <ProgressList steps={steps} done={done} />
        ) : own ? (
          <>
            <p className={styles.lead}>
              Hoista you run yourself, on a server or in Docker, at its own address. Chat apps and
              cloud agents reach it when this computer is off.
            </p>
            <SelectField
              label="Run it in"
              value={picked}
              options={whereChoices}
              onChange={(where) => {
                setChoice(where);
                setBefore(null);
              }}
              isEqual={sameWhere}
            />
            <TextField
              label="Its address"
              value={address}
              placeholder="https://hoista.example.com"
              hint={`Where you open it. For Hoista in Docker on this computer: ${LOCAL_REMOTE_ADDRESS}`}
              onChange={setAddress}
            />
            <div className={styles.seal}>
              <span className={styles.label}>Not running yet?</span>
              <span className={styles.hint}>
                The self-hosting guide covers Docker on a server, HTTPS in front of it, and claiming
                it with a passkey.
              </span>
              <div>
                <Button
                  onClick={() => {
                    void openExternal(SELF_HOST_GUIDE).then(() => {
                      setGuideOpened(true);
                    });
                  }}
                >
                  {guideOpened ? 'Opened the guide in your browser' : 'Read the self-hosting guide'}
                </Button>
              </div>
            </div>
          </>
        ) : (
          <>
            <p className={styles.lead}>
              Hoista runs in your own account at its own address, so chat apps and cloud agents
              reach it when this computer is off. Nothing runs on any server of ours.
            </p>

            <SelectField
              label="Run it in"
              value={picked}
              options={whereChoices}
              onChange={(where) => {
                setChoice(where);
                setBefore(null);
              }}
              isEqual={sameWhere}
            />
            {toConnect && (
              <p className={styles.hint}>
                {serviceName(toConnect)} opens here, and you come straight back to this step.
              </p>
            )}

            <dl className={styles.facts}>
              <div>
                <dt>What it runs</dt>
                <dd>
                  Hoista’s published image for this version, pinned by its digest, so it’s exactly
                  the release you can check. Plus a disk for its data and its own address.
                </dd>
              </div>
              <div>
                <dt>Your keys</dt>
                <dd>
                  Provider tokens are sealed on your remote, in a part of Hoista agents never reach.
                  Its key stays a secret at your provider. Nobody at Hoista can see either.
                </dd>
              </div>
              <div>
                <dt>Stays here for now</dt>
                <dd>
                  Your projects, accounts and agents, until you choose Push to send them there.
                </dd>
              </div>
              <div>
                <dt>Who pays</dt>
                <dd>Your provider bills you for it directly, like any service you run there.</dd>
              </div>
            </dl>
          </>
        )}
        <ErrorMessage error={setUp.error ?? connectServer.error ?? providers.error} />
      </div>
    </Dialog>
  );
}

export function RemoveRemoteDialog({
  remote,
  onClose,
}: {
  remote: RemoteStatus;
  onClose: () => void;
}) {
  const forget = useForgetServer();
  const place = remotePlace(remote.target);
  return (
    <Dialog
      title="Stop using your Hoista in the cloud?"
      eyebrow="Instances"
      onClose={forget.isPending ? () => undefined : onClose}
      actions={
        <>
          <Button variant="ghost" disabled={forget.isPending} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="accent"
            busy={forget.isPending}
            onClick={() => {
              forget.mutate(undefined, { onSuccess: onClose });
            }}
          >
            {forget.isPending ? 'Forgetting' : 'Forget it here'}
          </Button>
        </>
      }
    >
      <DialogText>
        <p>This computer stops showing it. Hoista on this computer keeps working as before.</p>
        <ul>
          <li>
            It keeps running in {place} until you delete the hoista project there, in {place}’s
            dashboard.
          </li>
          {remote.url ? <li>Its address is {remote.url}.</li> : null}
        </ul>
        <ErrorMessage error={forget.error} />
      </DialogText>
    </Dialog>
  );
}

export function StopEverythingDialog({ onClose }: { onClose: () => void }) {
  const stop = useStopEverything();
  const agents = useAgents();
  const count = agents.data?.items.length ?? 0;

  return (
    <Dialog
      title="Cut off every agent?"
      eyebrow="Settings · Safety"
      onClose={stop.isPending ? () => undefined : onClose}
      actions={
        <>
          <Button variant="ghost" disabled={stop.isPending} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="danger"
            busy={stop.isPending}
            busyLabel="Cutting off"
            onClick={() => {
              stop.mutate(undefined, { onSuccess: onClose });
            }}
          >
            Cut off every agent
          </Button>
        </>
      }
    >
      <DialogText>
        <p>Use this if something looks wrong. Your projects keep running.</p>
        <ul>
          <li>
            {count > 0
              ? `${plural(count, 'agent')} lose access, including coding tools. Each must be approved again to reconnect.`
              : 'Every agent loses access and must be approved again to reconnect.'}
          </li>
          <li>Agent access is paused, so nothing new can connect until you turn it back on.</li>
          <li>Waiting connection requests are dropped. It’s recorded in activity.</li>
          <li>
            Providers aren’t changed. If a provider token may have leaked, revoke it at the
            provider, then reconnect the account here.
          </li>
        </ul>
        <ErrorMessage error={stop.error} />
      </DialogText>
    </Dialog>
  );
}
