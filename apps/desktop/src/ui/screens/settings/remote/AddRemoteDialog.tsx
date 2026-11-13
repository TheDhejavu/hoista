/**
 * Adding a remote: another Hoista of yours the app can show and switch to,
 * the way apps add a server elsewhere (Jellyfin, Home Assistant): give its
 * address, then sign in in the browser. One dialog that never closes
 * mid-task:
 *
 * 1. Its address (and a name). Don't have one yet? Run one yourself (the
 *    few commands, each copyable) or have Hoista set one up in your
 *    Railway or Fly.io account (that opens over this one).
 * 2. Sign in in the browser: its page opens; you set it up the first time
 *    (claim code and passkey) or sign in with your passkey, then click
 *    Allow. The dialog follows along by itself. No code to compare.
 * 3. Done: switch to it now, or stay on this Mac. When the remote is empty
 *    and this Mac has projects, it first offers to push them over, once.
 *
 * Started for a remote already in the list (`signInTo`), it opens at step 2.
 */
import { useState } from 'react';
import type { ProviderId } from '@shared/api';
import { Check, Cloud, Container, Globe, Server } from 'lucide-react';
import {
  useAddRemote,
  usePushToRemote,
  useRemoteContents,
  useRemotes,
  useServerProviders,
  useSettings,
  useShowRemote,
  useSignInRemote,
  useSignOutRemote,
} from '@ui/api/settings';
import { openExternal } from '@ui/api/system';
import { Button } from '@ui/components/Button';
import { CopyField } from '@ui/components/CopyField';
import { Dialog, DialogText } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { TextField } from '@ui/components/Field';
import { SelectField, type Option } from '@ui/components/Select';
import { cx } from '@ui/lib/cx';
import { serviceName } from '@shared/providers';
import { SetUpRemoteDialog } from './RemoteDialogs';
import { PushPanel, pushChoice } from './PushOffer';
import styles from './AddRemoteDialog.module.css';
import { LOCAL_REMOTE_ADDRESS, REMOTE_PORT } from '@shared/ports';

type Step = 'guide' | 'address' | 'sign_in';
type Where = 'docker_mac' | 'docker_server' | 'hoist_cli';

const SOURCE = 'https://github.com/TheDhejavu/hoista.git';

const WHERE_CHOICES: Option<Where | null>[] = [
  { value: null, label: 'None' },
  { value: 'docker_mac', label: 'Docker on this Mac' },
  { value: 'docker_server', label: 'Docker on a server of yours' },
  { value: 'hoist_cli', label: 'A provider account, with the hoist command line' },
];

interface Command {
  title: string;
  text: string;
  /** A command to copy, or a page to open. */
  copy?: string;
  open?: string;
}

const GUIDES: Record<Where, Command[]> = {
  docker_mac: [
    {
      title: 'Get Hoista',
      text: 'In Terminal, in the folder you keep code in.',
      copy: `git clone ${SOURCE} && cd hoista`,
    },
    {
      title: 'Start it',
      text: `Builds Hoista, makes its key, starts it at localhost:${String(REMOTE_PORT)} and prints a claim code. The first build takes a while.`,
      copy: './deploy/server/start.sh',
    },
    {
      title: 'Come back and sign in',
      text: 'The first time, its page asks for the claim code the script printed, then you make a passkey.',
    },
  ],
  docker_server: [
    {
      title: 'Get Hoista on the server',
      text: 'Over SSH, on the server.',
      copy: `git clone ${SOURCE} && cd hoista`,
    },
    {
      title: 'Start it at its own address',
      text: `Use the HTTPS address it will answer on. It listens on port ${String(REMOTE_PORT)} of the server.`,
      copy: 'HOISTA_PUBLIC_URL=https://hoista.example.com ./deploy/server/start.sh',
    },
    {
      title: 'Put HTTPS in front',
      text: `Point that address at port ${String(REMOTE_PORT)} through a reverse proxy such as Caddy, or a tunnel. Passkeys need HTTPS.`,
    },
    {
      title: 'Come back and sign in',
      text: 'The first time, its page asks for the claim code the script printed, then you make a passkey.',
    },
  ],
  hoist_cli: [
    {
      title: 'Create it',
      text: 'In Terminal. It asks which connected account to run it in, builds it, prints its claim code and opens its page to claim it with a passkey. Put fly in place of railway to run it there instead.',
      copy: 'hoist remote create railway',
    },
    {
      title: 'Come back and sign in',
      text: 'It’s in your remotes now. Sign in, then allow this app with your passkey.',
    },
  ],
};

export function AddRemoteDialog({
  onClose,
  signInTo,
}: {
  onClose: () => void;
  /** A remote already in the list, to sign this computer in to. */
  signInTo?: string;
}) {
  const [step, setStep] = useState<Step>(signInTo ? 'sign_in' : 'address');
  const [where, setWhere] = useState<Where | null>('docker_mac');
  const [address, setAddress] = useState('');
  const [name, setName] = useState('');
  const [remoteId, setRemoteId] = useState<string | null>(signInTo ?? null);
  const [settingUp, setSettingUp] = useState(false);
  const remotes = useRemotes();
  const providers = useServerProviders();
  const settings = useSettings().data;
  const add = useAddRemote();
  const signIn = useSignInRemote();
  const cancel = useSignOutRemote();
  const show = useShowRemote();
  const push = usePushToRemote();
  // Declined the push: the usual switch-or-stay choice instead.
  const [declined, setDeclined] = useState(false);

  const data = remotes.data;
  const remote = data?.items.find((item) => item.id === remoteId) ?? null;
  const waiting = data?.waiting?.remote_id === remoteId ? data.waiting : null;
  const problem = data?.problem?.remote_id === remoteId ? data.problem.message : null;
  // Signed in, with the remote it's about in hand.
  const signedIn = remote !== null && remote.signed_in;
  const contents = useRemoteContents(signedIn ? remote.id : null);
  // The offer stands while the remote is empty, this Mac has something, and
  // nothing has been pushed or declined yet.
  const offering =
    signedIn &&
    !declined &&
    !push.data &&
    contents.data !== undefined &&
    pushChoice(contents.data) === 'offer';
  const offered = providers.data?.providers ?? [];

  function connect(): void {
    const before = new Set((data?.items ?? []).map((item) => item.id));
    add.mutate(
      { url: address, ...(name.trim() ? { name: name.trim() } : {}) },
      {
        onSuccess: (view) => {
          const added = view.items.find((item) => !before.has(item.id));
          if (added) setRemoteId(added.id);
          setStep('sign_in');
        },
      },
    );
  }

  /** Leaves the dialog; a sign-in still waiting is cancelled, not left running. */
  function close(): void {
    if (remoteId && waiting) cancel.mutate(remoteId);
    onClose();
  }

  const title =
    step === 'guide'
      ? 'Run Hoista yourself'
      : step === 'address'
        ? 'Add a remote'
        : signedIn
          ? `${remote.name} is connected`
          : `Sign in to ${remote?.name ?? 'your remote'}`;

  // Setting one up is a step of this same window: back returns to the address.
  if (settingUp && settings) {
    return (
      <Dialog shell title={title} onClose={close}>
        <SetUpRemoteDialog
          depth={1}
          backLabel="Add a remote"
          onBack={() => {
            setSettingUp(false);
          }}
          onClose={close}
          onConnectAgent={() => {
            setSettingUp(false);
          }}
        />
      </Dialog>
    );
  }

  return (
    <Dialog
      depth={step === 'address' ? 0 : step === 'sign_in' && signedIn ? 2 : 1}
      title={title}
      eyebrow="Instances"
      icon={<Server size={18} aria-hidden="true" />}
      onClose={close}
      {...back()}
      actions={actions()}
    >
      <div className={styles.body}>
        {step !== 'guide' && (
          <Progress
            steps={['Its address', 'Sign in', 'Switch to it']}
            done={step === 'sign_in' ? (signedIn ? 2 : 1) : 0}
          />
        )}
        {step === 'guide' && (
          <>
            <SelectField
              label="Where it runs"
              value={where}
              options={WHERE_CHOICES}
              onChange={setWhere}
            />
            {where ? (
              <ol className={styles.guide}>
                {GUIDES[where].map((command, index) => (
                  <li key={command.title} className={styles.guideStep}>
                    <span className={styles.guideNumber} aria-hidden="true">
                      {index + 1}
                    </span>
                    <div className={styles.guideBody}>
                      <span className={styles.guideTitle}>{command.title}</span>
                      <span className={styles.hint}>{command.text}</span>
                      {command.copy && <CopyField text={command.copy} label="the command" />}
                      {command.open && (
                        <div>
                          <Button
                            onClick={() => {
                              void openExternal(command.open ?? '');
                            }}
                          >
                            Open {command.open.replace(/^https?:\/\//, '')}
                          </Button>
                        </div>
                      )}
                    </div>
                  </li>
                ))}
              </ol>
            ) : (
              <p className={styles.hint}>Pick where it runs to see the steps.</p>
            )}
            <p className={styles.hint}>The full guide is at docs.hoista.dev/guides/self-host.</p>
          </>
        )}
        {step === 'address' && (
          <>
            <DialogText>
              A remote is another Hoista of yours, on a server or in Docker. Give the address you
              open it at; you’ll sign in to it in your browser.
            </DialogText>
            <TextField
              label="Address"
              value={address}
              placeholder="https://hoista.example.com"
              hint={`For Hoista in Docker on this Mac: ${LOCAL_REMOTE_ADDRESS}`}
              onChange={setAddress}
            />
            <TextField
              label="Name"
              value={name}
              placeholder="home-server"
              hint="What you call it in the switcher. Leave it empty to use its address."
              onChange={setName}
            />
          </>
        )}
        {step === 'address' && (
          <How onPick={pick} canSetUp={offered.length > 0} providers={offered} />
        )}
        {step === 'sign_in' && !signedIn && (
          <div className={styles.browser} aria-live="polite">
            <span className={cx(styles.browserIcon, waiting && styles.busy)} aria-hidden="true">
              <Globe size={20} />
            </span>
            <span className={styles.guideTitle}>
              {waiting
                ? 'Finish in your browser'
                : problem
                  ? 'The sign-in didn’t finish'
                  : signIn.isPending
                    ? 'Opening your browser'
                    : `Sign in to ${remote?.name ?? 'your remote'}`}
            </span>
            <p className={styles.hint}>
              {waiting
                ? 'Its page opened in your browser. Sign in with your passkey, then click Allow. This updates by itself.'
                : (problem ??
                  'Its page opens in your browser. Sign in with your passkey, then click Allow.')}
            </p>
            {waiting && (
              <p className={styles.small}>
                New server? Its page asks for the claim code from its log first.
              </p>
            )}
          </div>
        )}
        {step === 'sign_in' && signedIn && (
          <div className={styles.doneCard}>
            <span className={styles.doneIcon} aria-hidden="true">
              <Check size={16} />
            </span>
            <div>
              <span className={styles.guideTitle}>{remote.name}</span>
              <span className={cx(styles.hint, 'mono')}>
                {remote.url.replace(/^https?:\/\//, '')}
              </span>
              <p className={styles.hint}>
                Signed in. Switch between it and this Mac any time from the menu at the bottom left.
              </p>
            </div>
          </div>
        )}
        {step === 'sign_in' && signedIn && !declined && (
          <PushPanel remoteId={remote.id} name={remote.name} done={push.data} error={push.error} />
        )}
        <ErrorMessage error={add.error ?? signIn.error ?? cancel.error ?? remotes.error} />
      </div>
    </Dialog>
  );

  /** Back to the address from a step opened from it; never once it's connected. */
  function back(): { onBack?: () => void; backLabel?: string } {
    if (step === 'guide') {
      return {
        onBack: () => {
          setStep('address');
        },
        backLabel: 'Add a remote',
      };
    }
    if (step === 'sign_in' && !signedIn && !signInTo) {
      return {
        onBack: () => {
          // A sign-in still waiting is cancelled, not left running.
          if (remoteId && waiting) cancel.mutate(remoteId);
          setStep('address');
        },
        backLabel: 'Change the address',
      };
    }
    return {};
  }

  function pick(choice: 'run' | 'for_me'): void {
    if (choice === 'run') setStep('guide');
    else setSettingUp(true);
  }

  function actions() {
    if (step === 'guide') {
      return (
        <>
          <Button variant="ghost" onClick={close}>
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={where === null}
            onClick={() => {
              if (where === 'docker_mac' && !address) setAddress(LOCAL_REMOTE_ADDRESS);
              setStep('address');
            }}
          >
            Continue
          </Button>
        </>
      );
    }
    if (step === 'address') {
      return (
        <>
          <Button variant="ghost" disabled={add.isPending} onClick={close}>
            Cancel
          </Button>
          <Button
            variant="primary"
            busy={add.isPending}
            busyLabel="Checking it answers"
            disabled={!address.trim()}
            onClick={connect}
          >
            Sign in
          </Button>
        </>
      );
    }
    if (signedIn && offering) {
      return (
        <>
          <Button
            variant="ghost"
            disabled={push.isPending}
            onClick={() => {
              setDeclined(true);
            }}
          >
            Not now
          </Button>
          <Button
            variant="primary"
            busy={push.isPending}
            busyLabel="Pushing"
            onClick={() => {
              push.mutate(remote.id);
            }}
          >
            Push to {remote.name}
          </Button>
        </>
      );
    }
    if (signedIn) {
      return (
        <>
          <Button variant="ghost" disabled={contents.isPending && !declined} onClick={onClose}>
            {push.data || contents.data?.remote.projects
              ? 'Keep using this Mac'
              : 'Stay on this Mac'}
          </Button>
          <Button
            variant="primary"
            busy={show.isPending}
            busyLabel="Switching"
            disabled={contents.isPending && !declined}
            onClick={() => {
              show.mutate(remote.id, { onSuccess: onClose });
            }}
          >
            Switch to {remote.name}
          </Button>
        </>
      );
    }
    return (
      <>
        <Button variant="ghost" onClick={close}>
          Cancel
        </Button>
        {waiting ? (
          <Button
            onClick={() => {
              void openExternal(waiting.link);
            }}
          >
            Open the page again
          </Button>
        ) : (
          <Button
            variant="primary"
            busy={signIn.isPending}
            busyLabel="Opening its page"
            disabled={!remoteId}
            onClick={() => {
              if (remoteId) signIn.mutate(remoteId);
            }}
          >
            {problem ? 'Try again' : 'Sign in'}
          </Button>
        )}
      </>
    );
  }
}

/** For someone without a remote yet: the two ways to get one. */
function How({
  onPick,
  canSetUp,
  providers,
}: {
  onPick: (choice: 'run' | 'for_me') => void;
  canSetUp: boolean;
  providers: ProviderId[];
}) {
  const choices = [
    {
      key: 'run' as const,
      icon: <Container size={16} aria-hidden="true" />,
      title: 'Run one yourself',
      text: 'With Docker on this Mac or a server, or the hoist command line.',
      disabled: false,
    },
    {
      key: 'for_me' as const,
      icon: <Cloud size={16} aria-hidden="true" />,
      title: 'Set one up for me',
      text: canSetUp
        ? `In your ${providers.map(serviceName).join(' or ')} account.`
        : 'Connect a provider that runs services in Accounts first.',
      disabled: !canSetUp,
    },
  ];
  return (
    <div className={styles.other}>
      <span className={styles.otherHead}>Don’t have one yet?</span>
      <div className={styles.choices}>
        {choices.map((choice) => (
          <button
            key={choice.key}
            type="button"
            className={styles.choice}
            disabled={choice.disabled}
            onClick={() => {
              onPick(choice.key);
            }}
          >
            <span className={styles.choiceIcon}>{choice.icon}</span>
            <span className={styles.choiceText}>
              <span className={styles.guideTitle}>{choice.title}</span>
              <span className={styles.hint}>{choice.text}</span>
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}

function Progress({ steps, done }: { steps: string[]; done: number }) {
  return (
    <ol className={styles.progress} aria-live="polite">
      {steps.map((label, index) => {
        const state = index < done ? 'done' : index === done ? 'doing' : 'todo';
        return (
          <li key={label} className={cx(styles.progressItem, styles[state])}>
            <span className={styles.progressIcon} aria-hidden="true">
              {state === 'done' ? <Check size={12} /> : index + 1}
            </span>
            {label}
          </li>
        );
      })}
    </ol>
  );
}
