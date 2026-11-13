/**
 * Settings · Agents · Your remote: where Hoista keeps running when this
 * computer is off. Set up from a dropdown; once running, its status with
 * Open, and moving, pushing or turning it off under ⋮.
 */
import { Cloud } from 'lucide-react';
import { useState } from 'react';
import type { ProviderId, RemoteStatus, RemoteTarget } from '@shared/api';
import { useAccounts } from '@ui/api/accounts';
import { useRemoteContents, useRemotes, useServerProviders } from '@ui/api/settings';
import { serviceName } from '@shared/providers';
import { openExternal } from '@ui/api/system';
import { Button } from '@ui/components/Button';
import { Dot, type Tone } from '@ui/components/Dot';
import { RowMenu, type RowMenuItem } from '@ui/components/RowMenu';
import { SelectRow, SettingGroup, SettingRow } from '@ui/components/SettingRow';
import { remoteChoices, remoteHint, remoteName } from '@ui/lib/remote';
import { remotePlace } from '@ui/lib/remoteProviders';
import { timeAgo } from '@ui/lib/format';
import { MoveToServerDialog } from './remote/MoveToServerDialog';
import { pushChoice } from './remote/PushOffer';
import { RemoveRemoteDialog, SetUpRemoteDialog } from './remote/RemoteDialogs';
import { GROUP_ICON } from './groupIcon';
import styles from './AgentSettings.module.css';
import { ConnectServiceDialog } from '@ui/screens/accounts/ConnectDialog';
import { ConnectAgentDialog } from '@ui/screens/agents/ConnectAgentDialog';

type Flow = 'turn_on' | 'turn_off' | null;

export function RemoteSettings({ remote }: { remote: RemoteStatus }) {
  const [flow, setFlow] = useState<Flow>(null);
  const [nextTarget, setNextTarget] = useState<RemoteTarget | null>(null);
  // Connecting a provider, pushing, or an agent once the remote is ready, opens over Settings.
  const [connecting, setConnecting] = useState<ProviderId | null>(null);
  const [connectingAgent, setConnectingAgent] = useState(false);
  const [pushing, setPushing] = useState(false);
  const view = useRemotes();
  const providers = useServerProviders();
  const accounts = useAccounts();
  // Only providers you've connected that can run Hoista; connecting one comes first.
  const offered = providers.data?.providers ?? [];
  const connected = offered.filter((provider) =>
    (accounts.data?.items ?? []).some((account) => account.service === provider),
  );
  // The same remote in your remotes list, once this computer is signed in to it.
  const remoteUrl = remote.target !== 'this_computer' ? (remote.url ?? null) : null;
  const listed = remoteUrl
    ? (view.data?.items.find((item) => item.signed_in && sameUrl(item.url, remoteUrl)) ?? null)
    : null;
  const contents = useRemoteContents(listed?.id ?? null);
  const canPush = contents.data ? pushChoice(contents.data) === 'offer' : false;

  // These settings come from the Hoista being shown; a remote is set up from this computer.
  if (view.data && view.data.showing !== 'this_computer') {
    return null;
  }

  function moveTo(target: RemoteTarget): void {
    // Moving where Hoista runs is a real migration, so it goes through a dialog.
    if (target === remote.target) return;
    setNextTarget(target);
    setFlow(target === 'this_computer' ? 'turn_off' : 'turn_on');
  }

  const { target } = remote;
  return (
    <SettingGroup
      title="Your remote"
      description={
        target === 'this_computer'
          ? 'Hoista in an account of yours, so chat apps and cloud agents reach it when this computer is off.'
          : target === 'server'
            ? 'Hoista on your own server, so chat apps and cloud agents reach it when this computer is off.'
            : `Hoista in your own ${remotePlace(target)} account, so chat apps and cloud agents reach it when this computer is off.`
      }
      icon={<Cloud {...GROUP_ICON} />}
    >
      {target === 'this_computer' ? (
        <SelectRow
          label="Where Hoista runs"
          hint={remoteHint(target)}
          value={target}
          options={remoteChoices(connected, target)}
          onChange={moveTo}
        />
      ) : (
        <SettingRow label={`Remote · ${remoteName(target)}`} hint={<RemoteState remote={remote} />}>
          <span className={styles.controls}>
            {remote.url && (
              <Button
                onClick={() => {
                  if (remote.url) void openExternal(remote.url);
                }}
              >
                Open
              </Button>
            )}
            <RowMenu
              label="More for your remote"
              items={[
                ...(listed && canPush
                  ? [
                      {
                        label: 'Push this computer’s projects to it',
                        onSelect: () => {
                          setPushing(true);
                        },
                      },
                    ]
                  : []),
                ...connected
                  .filter((provider) => provider !== target)
                  .map<RowMenuItem>((provider) => ({
                    label: `Move to ${serviceName(provider)}`,
                    onSelect: () => {
                      moveTo(provider);
                    },
                  })),
                {
                  label: 'Turn off',
                  danger: true,
                  onSelect: () => {
                    moveTo('this_computer');
                  },
                },
              ]}
            />
          </span>
        </SettingRow>
      )}

      {target === 'this_computer' && connected.length === 0 && offered.length > 0 && (
        <SelectRow<ProviderId | null>
          label="Connect a provider first"
          hint={`Hoista runs in an account you connect, on a provider that runs services: ${offered.map(serviceName).join(' or ')}. It opens here.`}
          value={null}
          options={[
            { value: null, label: 'None' },
            ...offered.map((provider) => ({
              value: provider,
              label: `Connect ${serviceName(provider)}`,
            })),
          ]}
          onChange={(provider) => {
            setConnecting(provider);
          }}
        />
      )}
      {connecting && (
        <ConnectServiceDialog
          service={connecting}
          stay
          onClose={() => {
            setConnecting(null);
          }}
        />
      )}
      {flow === 'turn_on' && (
        <SetUpRemoteDialog
          initialTarget={nextTarget}
          onClose={() => {
            setFlow(null);
          }}
          onConnectAgent={() => {
            setFlow(null);
            setConnectingAgent(true);
          }}
        />
      )}
      {connectingAgent && (
        <ConnectAgentDialog
          onClose={() => {
            setConnectingAgent(false);
          }}
        />
      )}
      {flow === 'turn_off' && (
        <RemoveRemoteDialog
          remote={remote}
          onClose={() => {
            setFlow(null);
          }}
        />
      )}
      {pushing && listed && (
        <MoveToServerDialog
          remoteId={listed.id}
          provider={listed.name}
          onClose={() => {
            setPushing(false);
          }}
        />
      )}
    </SettingGroup>
  );
}

const STATE: Record<RemoteStatus['status'], { label: string; tone: Tone }> = {
  running: { label: 'Running', tone: 'good' },
  setting_up: { label: 'Setting up', tone: 'warn' },
  unreachable: { label: 'Not answering', tone: 'bad' },
  off: { label: 'Off', tone: 'off' },
};

/** How the remote is doing, and when it last synced with this computer. */
function RemoteState({ remote }: { remote: RemoteStatus }) {
  const state = STATE[remote.status];
  return (
    <span className={styles.status}>
      <Dot tone={state.tone} small />
      {state.label}
      {remote.last_sync ? ` · last synced with this computer ${timeAgo(remote.last_sync)}` : ''}
    </span>
  );
}

function sameUrl(a: string, b: string): boolean {
  return a.replace(/\/+$/, '') === b.replace(/\/+$/, '');
}
