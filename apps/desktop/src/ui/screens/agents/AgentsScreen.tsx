/**
 * Agents: the AI tools connected to Hoista, what each may do, anything
 * asking to connect, and what the agents did lately.
 */
import { useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { useConnectionRequests } from '@ui/api/agents';
import { Page } from '@ui/components/Page';
import { AgentActivityList } from './AgentActivityList';
import { AgentsTable } from './AgentsTable';
import { ConnectionRequestBanner } from './Banners';
import { ConnectAgentDialog } from './ConnectAgentDialog';
import styles from './AgentsScreen.module.css';

export function AgentsScreen() {
  // `?connect=1` opens the Connect an agent dialog straight away, for links from other screens.
  const [params] = useSearchParams();
  const [isConnecting, setConnecting] = useState(params.has('connect'));
  const requests = useConnectionRequests();

  return (
    <div className={styles.page}>
      <Page>
        <h1 className="visually-hidden">Agents</h1>

        {requests.data?.items.map((request) => (
          <ConnectionRequestBanner key={request.id} request={request} />
        ))}

        <AgentsTable
          onConnect={() => {
            setConnecting(true);
          }}
        />
        <AgentActivityList />
      </Page>

      {isConnecting && (
        <ConnectAgentDialog
          onClose={() => {
            setConnecting(false);
          }}
        />
      )}
    </div>
  );
}
