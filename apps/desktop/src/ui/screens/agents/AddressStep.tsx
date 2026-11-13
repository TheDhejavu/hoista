import { useEffect, useRef, useState } from 'react';
import { Check, LoaderCircle } from 'lucide-react';
import type { Agent, ConnectionRequest } from '@shared/api';
import { useAgentLink, useAgents, useConnectionRequests } from '@ui/api/agents';
import { openExternal } from '@ui/api/system';
import { AgentIcon } from '@ui/components/AgentIcon';
import { Button } from '@ui/components/Button';
import { CopyField } from '@ui/components/CopyField';
import { Dialog } from '@ui/components/Dialog';
import { Step, Steps } from '@ui/components/Steps';
import { AnswerRequest, requestWhere } from './Banners';
import { type Client } from './clients';
import styles from './AddressStep.module.css';
import shared from './connectAgent.module.css';
import { CLOCK_SLACK_MS, belongsTo, useConnectingHere } from './ConnectAgentDialog';
import { PortBusy } from './DoneStep';

export function AddressStep({
  client,
  since,
  onBack,
  onClose,
  onConnected,
}: {
  client: Client;
  since: number;
  onBack: () => void;
  onClose: () => void;
  /** Allowed on Hoista's page, or already allowed and using Hoista: done. */
  onConnected: (agent: Agent) => void;
}) {
  const link = useAgentLink();
  const requests = useConnectionRequests({ watch: true });
  const agents = useAgents({ watch: true });
  useConnectingHere(client.name);
  const mcp = link.data?.mcp ?? null;
  const host = link.data?.url?.replace(/^https?:\/\//, '') ?? null;
  const isThisClient = (name: string) => belongsTo(client.name, name);
  // This client signing in after the step opened; another agent's request isn't its.
  const arrived =
    requests.data?.items.find(
      (request) =>
        request.via === 'link' &&
        isThisClient(request.claimed_name) &&
        Date.parse(request.created_at) >= since - CLOCK_SLACK_MS,
    ) ?? null;
  // Allowed before, and talking to Hoista through the link already.
  const already =
    agents.data?.items.find(
      (agent) => agent.transport === 'link' && isThisClient(agent.name) && agent.last_seen,
    ) ?? null;
  const usedNow = already?.last_seen
    ? Date.parse(already.last_seen) >= since - CLOCK_SLACK_MS
    : false;

  // The sign-in this step saw, kept once it leaves the list: answered, or ended.
  const [signingIn, setSigningIn] = useState<ConnectionRequest | null>(null);
  if (arrived && arrived.id !== signingIn?.id) setSigningIn(arrived);
  const ended =
    signingIn !== null &&
    requests.data !== undefined &&
    !requests.data.items.some((item) => item.id === signingIn.id);
  const [opened, setOpened] = useState(false);

  useEffect(() => {
    if (!signingIn && already && usedNow) onConnected(already);
  }, [signingIn, already, usedNow, onConnected]);

  // Allowed on Hoista's page in the browser instead of here: it shows up as a
  // new agent without its request ever reaching this step. Any agent that
  // connects through the tunnel while this is open counts, not only the one
  // picked: whoever it is, it's connected, and this says so by its name.
  const before = useRef<Set<string> | null>(null);
  useEffect(() => {
    const items = agents.data?.items;
    if (!items) return;
    if (!before.current) {
      before.current = new Set(items.map((agent) => agent.id));
      return;
    }
    const known = before.current;
    const added = items.find((agent) => agent.transport === 'link' && !known.has(agent.id));
    if (added) onConnected(added);
  }, [agents.data, onConnected]);

  if (client.connect.kind !== 'link') return null;
  return (
    <Dialog
      depth={2}
      title={`Connect ${client.name}`}
      eyebrow="Through this computer"
      onBack={onBack}
      backLabel="Tunnel"
      onClose={onClose}
      actions={
        <Button large onClick={onClose}>
          Cancel
        </Button>
      }
    >
      <div className={shared.body}>
        {link.data && !link.data.listening ? (
          <PortBusy />
        ) : (
          host &&
          link.data?.seen_at && (
            <span className={styles.seen}>
              <span className={styles.tick} aria-hidden="true">
                <Check size={13} />
              </span>
              Tunnel reaches Hoista
            </span>
          )
        )}
        <Steps>
          <Step number={1} title="Copy Hoista’s address">
            <CopyField text={mcp ?? ''} label="Hoista’s address" />
          </Step>
          <Step number={2} title={`Add it in ${client.name}`}>
            <span className={shared.hint}>{client.connect.steps}</span>
          </Step>
        </Steps>
        {signingIn ? (
          <div className={styles.signingIn} aria-live="polite">
            <div className={styles.signingHead}>
              <AgentIcon name={signingIn.claimed_name} />
              <span className={styles.requestText}>
                <b>{client.name} is signing in</b>
                <span>
                  {ended
                    ? 'It was declined on the page, or it expired. Add Hoista in it again to start over.'
                    : 'Hoista opened its page in your browser. Check the code matches, then click Authorize there.'}
                </span>
              </span>
            </div>
            {signingIn.code && !ended && (
              <div className={styles.code}>
                <span>Code</span>
                <code>{signingIn.code}</code>
              </div>
            )}
            {!ended && (
              <div className={styles.signingFoot}>
                <span className={styles.waitingLine}>
                  <LoaderCircle size={14} className={shared.spin} aria-hidden="true" />
                  Waiting for you to authorize it
                </span>
                <Button
                  disabled={!signingIn.page}
                  onClick={() => {
                    if (!signingIn.page) return;
                    void openExternal(signingIn.page).then(() => {
                      setOpened(true);
                    });
                  }}
                >
                  {opened ? 'Open the page again' : 'Open the page'}
                </Button>
              </div>
            )}
          </div>
        ) : (
          <span className={styles.waitingBox}>
            <LoaderCircle size={15} className={shared.spin} aria-hidden="true" />
            {already
              ? `${client.name} is already connected. Ask it something and this finishes by itself.`
              : `Waiting for ${client.name} to sign in.`}
          </span>
        )}
        <OtherRequests
          requests={(requests.data?.items ?? []).filter(
            (request) => !isThisClient(request.claimed_name),
          )}
        />
      </div>
    </Dialog>
  );
}

/**
 * Other agents asking to connect while this dialog is open. The cards that
 * show them on every screen wait while a dialog is open, so they're shown
 * here instead, each with its code and the way to answer it.
 */
function OtherRequests({ requests }: { requests: ConnectionRequest[] }) {
  if (requests.length === 0) return null;
  return (
    <div className={styles.others} aria-live="polite">
      {requests.map((request) => (
        <div key={request.id} className={styles.other}>
          <AgentIcon name={request.claimed_name} />
          <span className={styles.requestText}>
            <b>{request.claimed_name} also wants to connect</b>
            <span>
              {request.answer_in === 'browser'
                ? requestWhere(request)
                : 'Review it in Agents once you close this.'}
            </span>
          </span>
          {request.code && (
            <span className={styles.otherCode} title="Matches the code on Hoista’s page">
              <span>Code</span>
              <code>{request.code}</code>
            </span>
          )}
          {request.answer_in === 'browser' && <AnswerRequest request={request} />}
        </div>
      ))}
    </div>
  );
}
