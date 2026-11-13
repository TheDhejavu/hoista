/**
 * Agents asking to connect, on every screen: a card at the bottom right with
 * who it is, where it came from, the code Hoista's browser page shows, and
 * the way to answer. The Agents page has its own banners, so the cards stay
 * off it, and they wait while a dialog is open. The Mac notification says the
 * same when the window is in the background. Once one is approved, a note
 * says it's connected.
 */
import { X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useLocation } from 'react-router-dom';
import type { ConnectionRequest } from '@shared/api';
import { agentDisplayName } from '@shared/agentLogos';
import { useAgents, useConnectionRequests } from '@ui/api/agents';
import { AgentIcon } from '@ui/components/AgentIcon';
import { useConnectingNow } from '@ui/lib/connectingNow';
import { showToast } from '@ui/lib/toast';
import { useModalOpen } from '@ui/lib/useModalOpen';
import { AnswerRequest, requestWhere } from '@ui/screens/agents/Banners';
import styles from './RequestCards.module.css';
import { SECOND_MS } from '@shared/time';

/** How long a declined or expired request is watched for before it's dropped. */
const GIVE_UP_AFTER_MS = 20 * SECOND_MS;

/** More than this at once, and the rest wait on the Agents page. */
const SHOWN = 3;

export function RequestCards() {
  const { pathname } = useLocation();
  const requests = useConnectionRequests({ watch: true }).data?.items ?? [];
  const [hidden, setHidden] = useState<string[]>([]);
  const connecting = useConnectingNow();
  // While a dialog is open the cards can't be clicked, and a click would close
  // the dialog: they wait until it closes.
  const modalOpen = useModalOpen();
  const shown =
    pathname.startsWith('/agents') || modalOpen
      ? []
      : requests
          .filter(
            (request) =>
              !hidden.includes(request.id) &&
              !(connecting && agentDisplayName(request.claimed_name) === connecting),
          )
          .slice(0, SHOWN);
  const stack = useRef<HTMLDivElement>(null);
  const ids = shown.map((request) => request.id).join(',');
  useConnectedNotes(requests, connecting);

  // In the top layer, like the toasts, raised again for each new request.
  useEffect(() => {
    const element = stack.current;
    if (!element || typeof element.showPopover !== 'function') return;
    if (element.matches(':popover-open')) element.hidePopover();
    if (ids) element.showPopover();
  }, [ids]);

  return (
    <div
      ref={stack}
      popover="manual"
      className={styles.stack}
      aria-label="Agents asking to connect"
    >
      {shown.map((request) => (
        <RequestCard
          key={request.id}
          request={request}
          onHide={() => {
            setHidden((list) => [...list, request.id]);
          }}
        />
      ))}
    </div>
  );
}

function RequestCard({ request, onHide }: { request: ConnectionRequest; onHide: () => void }) {
  return (
    <section className={styles.card} role="status" aria-live="polite">
      <div className={styles.head}>
        <span className={styles.icon}>
          <AgentIcon name={request.claimed_name} />
        </span>
        <span className={styles.text}>
          <strong>{request.claimed_name} wants to connect</strong>
          <span>{requestWhere(request)}</span>
        </span>
        <button type="button" className={styles.close} aria-label="Hide" onClick={onHide}>
          <X size={14} aria-hidden="true" />
        </button>
      </div>
      <div className={styles.foot}>
        {request.code && (
          <span className={styles.code} title="Matches the code on Hoista’s page in your browser">
            <span>Code</span>
            <code>{request.code}</code>
          </span>
        )}
        <AnswerRequest request={request} />
      </div>
    </section>
  );
}

/**
 * Says when a sign-in it saw comes through: once a request leaves the list and
 * its agent is connected, "Claude is connected". The Connect dialog shows its
 * own, so its agent is left to it.
 */
function useConnectedNotes(requests: ConnectionRequest[], connecting: string | null): void {
  const [seen, setSeen] = useState<ConnectionRequest[]>([]);
  // Agents whose request just left the list: allowed, declined or expired.
  const [left, setLeft] = useState<string[]>([]);
  const ids = requests.map((request) => request.id).join(',');
  if (ids !== seen.map((request) => request.id).join(',')) {
    const gone = seen.filter((old) => !requests.some((request) => request.id === old.id));
    setSeen(requests);
    if (gone.length > 0) {
      setLeft((list) => [
        ...list,
        ...gone.map((request) => agentDisplayName(request.claimed_name)),
      ]);
    }
  }
  const agents = useAgents({ watch: left.length > 0 }).data;

  useEffect(() => {
    if (left.length === 0) return;
    const items = agents?.items ?? [];
    const done = left.filter((name) => items.some((agent) => agent.name === name));
    for (const name of done) {
      if (name !== connecting) showToast(`${name} is connected`);
    }
    // Declined or expired: stop watching after a short while.
    const wait = setTimeout(
      () => {
        setLeft([]);
      },
      done.length > 0 ? 0 : GIVE_UP_AFTER_MS,
    );
    return () => {
      clearTimeout(wait);
    };
  }, [left, agents, connecting]);
}
