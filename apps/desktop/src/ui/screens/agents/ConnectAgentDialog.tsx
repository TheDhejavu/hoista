/**
 * Connect an agent: pick where you talk to AI, then follow its steps in this
 * one window. Coding tools get the command that adds Hoista. Chat apps and
 * cloud agents choose how they reach Hoista: this computer through a tunnel
 * (MCP over HTTP), or a remote. Through this computer, the window follows it
 * to the end: the tunnel, the address to paste, the agent signing in, and
 * allowing it here with Touch ID or the password.
 */
import { useEffect, useState } from 'react';
import type { AccessLevel, Settings } from '@shared/api';
import { agentDisplayName } from '@shared/agentLogos';
import { useAgentLink } from '@ui/api/agents';
import { useSettings } from '@ui/api/settings';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { QueryState } from '@ui/components/QueryState';
import { SelectField, type Option } from '@ui/components/Select';
import { setConnectingNow } from '@ui/lib/connectingNow';
import { SetUpRemoteDialog } from '@ui/screens/settings/remote/RemoteDialogs';
import { CLIENTS, type Client } from './clients';
import styles from './ConnectAgentDialog.module.css';
import shared from './connectAgent.module.css';
import { SECOND_MS } from '@shared/time';
import { TunnelStep } from './TunnelStep';
import { AddressStep } from './AddressStep';
import { CodingToolSteps, DoneStep, RemoteSteps } from './DoneStep';
import { type Route, RouteChoice } from './RouteChoice';

/** A sign-in stamped this much before the step opened still counts: the engine's clock and the page's can differ. */
export const CLOCK_SLACK_MS = 5 * SECOND_MS;

const CLIENT_CHOICES: Option<Client | null>[] = [
  { value: null, label: 'None' },
  ...CLIENTS.map((client) => ({
    value: client,
    label: client.name,
    group: client.group,
  })),
];

export const FIRST_QUESTION = 'what projects do I have on Hoista?';

export type Stage =
  | { at: 'pick' }
  | { at: 'tunnel' }
  | { at: 'address'; since: number }
  /** `agent`: the name of the agent that connected, which may not be the one picked. */
  | { at: 'done'; agent: string; access: AccessLevel; everyProject: boolean }
  | { at: 'remote' };

export function ConnectAgentDialog({
  onClose,
  initial = null,
}: {
  onClose: () => void;
  /** Where the person talks to AI, when it's already known (setup's Agents step). */
  initial?: Client | null;
}) {
  const settings = useSettings();
  return (
    <Dialog shell title="Connect an agent" onClose={onClose}>
      <QueryState query={settings}>
        {(data) => <ConnectFlow settings={data} initial={initial} onClose={onClose} />}
      </QueryState>
    </Dialog>
  );
}

function ConnectFlow({
  settings,
  initial,
  onClose,
}: {
  settings: Settings;
  initial: Client | null;
  onClose: () => void;
}) {
  // Nothing picked until you choose: the steps depend on where you talk to AI.
  const [client, setClient] = useState<Client | null>(initial);
  // Nothing picked until you choose how it reaches Hoista.
  const [route, setRoute] = useState<Route | null>(null);
  const [stage, setStage] = useState<Stage>({ at: 'pick' });
  const link = useAgentLink();
  const hasRemote = settings.agents.remote.target !== 'this_computer';
  const name = client?.name ?? '';

  function toPick(): void {
    setStage({ at: 'pick' });
  }
  function next(): void {
    if (route === 'remote') {
      // With a remote, its Hoista URL is shown on the first step already.
      if (!hasRemote) setStage({ at: 'remote' });
      return;
    }
    // A tunnel Hoista already knows goes straight to its address.
    if (link.data?.mcp) setStage({ at: 'address', since: Date.now() });
    else setStage({ at: 'tunnel' });
  }

  switch (stage.at) {
    case 'remote':
      return (
        <SetUpRemoteDialog
          depth={1}
          backLabel={`Connect ${name}`}
          onBack={toPick}
          onClose={onClose}
          onConnectAgent={toPick}
        />
      );
    case 'tunnel':
      return (
        <TunnelStep
          clientName={name}
          onBack={toPick}
          onClose={onClose}
          onAddress={() => {
            setStage({ at: 'address', since: Date.now() });
          }}
        />
      );
    case 'address':
      if (!client || client.connect.kind !== 'link') return null;
      return (
        <AddressStep
          client={client}
          since={stage.since}
          onBack={() => {
            setStage({ at: 'tunnel' });
          }}
          onClose={onClose}
          onConnected={(agent) => {
            setStage({
              at: 'done',
              agent: agentDisplayName(agent.name),
              access: agent.access,
              everyProject: agent.projects === null,
            });
          }}
        />
      );
    case 'done':
      return (
        <DoneStep
          clientName={stage.agent}
          access={stage.access}
          everyProject={stage.everyProject}
          onClose={onClose}
        />
      );
    case 'pick':
      break;
  }

  const continues = client?.connect.kind === 'link' && !(route === 'remote' && hasRemote);
  return (
    <Dialog
      depth={0}
      title="Connect an agent"
      onClose={onClose}
      actions={
        continues ? (
          <Button variant="accent" large disabled={!route} onClick={next}>
            Continue
          </Button>
        ) : (
          <Button variant="accent" large onClick={onClose}>
            Done
          </Button>
        )
      }
      sideAction={
        continues ? (
          <span className={shared.hint}>You approve {name} on this computer either way.</span>
        ) : undefined
      }
    >
      <div className={shared.body}>
        <SelectField
          label="Which agent do you use?"
          value={client}
          options={CLIENT_CHOICES}
          onChange={setClient}
        />
        {!client && <p className={styles.pick}>Pick one to see how to connect it.</p>}
        {client?.connect.kind === 'link' && (
          <RouteChoice
            clientName={client.name}
            route={route}
            onRoute={setRoute}
            settings={settings}
          />
        )}
        {client?.connect.kind === 'link' && route === 'remote' && hasRemote && (
          <RemoteSteps client={client} settings={settings} />
        )}
        {client && client.connect.kind !== 'link' && (
          <CodingToolSteps client={client} settings={settings} />
        )}
      </div>
    </Dialog>
  );
}

/**
 * Whether a name an agent registered with is this client's. Apps name
 * themselves loosely, so a name Hoista doesn't know as any client counts too;
 * a name that is another client's never does.
 */
export function belongsTo(clientName: string, registered: string): boolean {
  const name = agentDisplayName(registered);
  return name === clientName || !CLIENTS.some((client) => client.name === name);
}

/** While this step is open, its agent's request is answered here, not in the cards. */
export function useConnectingHere(name: string): void {
  useEffect(() => {
    setConnectingNow(name);
    return () => {
      setConnectingNow(null);
    };
  }, [name]);
}
