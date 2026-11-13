/**
 * Banners at the top of the Agents page for agents asking to connect, each
 * with its one way to answer.
 */
import { useState } from 'react';
import type { ConnectionRequest } from '@shared/api';
import { Button } from '@ui/components/Button';
import { AgentIcon } from '@ui/components/AgentIcon';
import { openExternal } from '@ui/api/system';
import { ConnectRequestDialog } from './ConnectRequestDialog';
import styles from './Banners.module.css';

/**
 * The one way to answer a request. A sign-in through the browser is approved
 * on Hoista's page there, since that page sends the browser back to the agent;
 * the app only opens it again if it was closed. A tool on this computer is
 * reviewed here.
 */
export function AnswerRequest({
  request,
  large = false,
  onOpened,
}: {
  request: ConnectionRequest;
  large?: boolean;
  onOpened?: () => void;
}) {
  const [isReviewing, setReviewing] = useState(false);
  const [opened, setOpened] = useState(false);

  if (request.answer_in === 'browser') {
    return (
      <Button
        large={large}
        disabled={!request.page}
        title={request.page ? undefined : 'Hoista’s page isn’t running. Restart Hoista.'}
        onClick={() => {
          if (!request.page) return;
          void openExternal(request.page).then(() => {
            setOpened(true);
            onOpened?.();
          });
        }}
      >
        {opened ? 'Open the page again' : 'Open the page'}
      </Button>
    );
  }
  return (
    <>
      <Button
        large={large}
        variant="accent"
        onClick={() => {
          setReviewing(true);
        }}
      >
        Review
      </Button>
      {isReviewing && (
        <ConnectRequestDialog
          request={request}
          onClose={() => {
            setReviewing(false);
          }}
        />
      )}
    </>
  );
}

/** Where it came from, and where it's answered. */
export function requestWhere(request: ConnectionRequest): string {
  const from = request.via === 'this_computer' ? 'A tool on this computer' : 'From the web';
  return request.answer_in === 'browser'
    ? `${from} · approve it on Hoista’s page in your browser`
    : `${from} · review it here`;
}

export function ConnectionRequestBanner({ request }: { request: ConnectionRequest }) {
  return (
    <div className={styles.banner} role="status">
      <span className={styles.icon}>
        <AgentIcon name={request.claimed_name} />
      </span>
      <span className={styles.bannerText}>
        <strong>{request.claimed_name} wants to connect</strong>
        <span className={styles.muted}>
          {requestWhere(request)}
          {request.code ? ` · code ${request.code}` : ''}
        </span>
      </span>
      <AnswerRequest request={request} large />
    </div>
  );
}
