/**
 * The page a chat link opens when an agent's request needs a provider that
 * Hoista can no longer reach, for example Claude asks to deploy and the
 * Railway access has run out. The request waits; you reconnect here, in one
 * click where the provider offers it or with a new token, and the agent carries
 * on by itself. Nothing secret goes through the chat.
 *
 * Route: `/reconnect/:service?agent=Claude&project=waitlist-v2&action=deploy`,
 * drawn over the Agents page.
 */
import { useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import type { ConnectGuide, TokenCheck } from '@shared/api';
import { isServiceId, serviceName } from '@shared/providers';
import { useConnectAccount, useConnectGuide } from '@ui/api/accounts';
import { AgentIcon } from '@ui/components/AgentIcon';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { SkeletonLines } from '@ui/components/Skeleton';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { TokenField } from '@ui/components/TokenField';
import { QueryState } from '@ui/components/QueryState';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { OrUseToken, SignInOption } from '@ui/components/SignInOption';
import { TokenChecks } from '@ui/components/TokenChecks';
import { ExternalLink } from '@ui/components/ExternalLink';
import { useRequiredParam } from '@ui/lib/useRequiredParam';
import { AgentsScreen } from './AgentsScreen';
import styles from './ReconnectScreen.module.css';

export function ReconnectScreen() {
  const service = useRequiredParam('service');
  const navigate = useNavigate();

  function close(): void {
    void navigate('/agents');
  }

  return (
    <>
      <AgentsScreen />
      {isServiceId(service) ? (
        <ReconnectFor service={service} onClose={close} />
      ) : (
        <Dialog title="Can’t reconnect that" onClose={close}>
          <p>Hoista doesn’t have an adapter for that service yet.</p>
        </Dialog>
      )}
    </>
  );
}

function ReconnectFor({
  service,
  onClose,
}: {
  service: Parameters<typeof useConnectGuide>[0];
  onClose: () => void;
}) {
  const guide = useConnectGuide(service);
  return (
    <QueryState
      query={guide}
      skeleton={
        <Dialog
          title={`Reconnect ${serviceName(service)}`}
          icon={<ServiceIcon id={service} size={18} />}
          onClose={onClose}
        >
          <SkeletonLines count={3} />
        </Dialog>
      }
    >
      {(data) => <ReconnectDialog guide={data} onClose={onClose} />}
    </QueryState>
  );
}

function ReconnectDialog({ guide, onClose }: { guide: ConnectGuide; onClose: () => void }) {
  const [params] = useSearchParams();
  const agent = params.get('agent') ?? 'Your agent';
  const project = params.get('project');
  const action = params.get('action') ?? 'deploy';
  const asked = project ? `${action} ${project}` : action;

  const connect = useConnectAccount();
  const [token, setToken] = useState('');
  const [checks, setChecks] = useState<TokenCheck[] | null>(null);
  const name = serviceName(guide.service);

  if (checks) {
    return (
      <Dialog
        title={`${name} is connected again`}
        icon={<ServiceIcon id={guide.service} size={18} />}
        onClose={onClose}
        actions={
          <Button variant="accent" large onClick={onClose}>
            Done
          </Button>
        }
      >
        <div className={styles.body}>
          <p className={styles.done}>
            {agent} carries on with the {action} by itself. You can go back to your chat.
          </p>
          <TokenChecks checks={checks} />
        </div>
      </Dialog>
    );
  }

  return (
    <Dialog
      title={`Reconnect ${name} so ${agent} can carry on`}
      icon={<ServiceIcon id={guide.service} size={18} />}
      onClose={onClose}
      actions={
        <>
          <Button large onClick={onClose}>
            Not now
          </Button>
          <Button
            variant="accent"
            large
            disabled={!token.trim() || connect.isPending}
            onClick={() => {
              connect.mutate(
                { service: guide.service, token },
                {
                  onSuccess: (result) => {
                    setChecks(result.checks);
                  },
                  onSettled: () => {
                    setToken('');
                  },
                },
              );
            }}
          >
            {connect.isPending ? 'Checking' : 'Verify & reconnect'}
          </Button>
        </>
      }
    >
      <div className={styles.body}>
        <div className={styles.request}>
          <AgentIcon name={agent} small />
          <span>
            <b>{agent}</b> asked to {asked}. Hoista’s access to {name} has run out, so the {action}{' '}
            is waiting, not cancelled.
          </span>
        </div>
        {guide.sign_in && (
          <>
            <SignInOption
              service={guide.service}
              details={guide.sign_in}
              again
              onConnected={setChecks}
            />
            <OrUseToken label="or paste a new token" />
          </>
        )}
        <TokenField
          label={`New ${name} token`}
          value={token}
          placeholder={guide.token_placeholder}
          onChange={setToken}
        />
        <ExternalLink url={guide.token_url}>Open {name}’s tokens page</ExternalLink>
        <ErrorMessage error={connect.error} />
      </div>
    </Dialog>
  );
}
