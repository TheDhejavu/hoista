/**
 * The agents list over Settings → Agents ("Manage agents"), so changing an
 * agent's access never closes Settings. Closing it goes back to Settings.
 */
import { useState } from 'react';
import { useConnectionRequests } from '@ui/api/agents';
import { Dialog } from '@ui/components/Dialog';
import { AgentsTable } from './AgentsTable';
import { ConnectionRequestBanner } from './Banners';
import { ConnectAgentDialog } from './ConnectAgentDialog';
import styles from './AgentsDialog.module.css';

export function AgentsDialog({ onClose }: { onClose: () => void }) {
  const [isConnecting, setConnecting] = useState(false);
  const requests = useConnectionRequests();

  return (
    <Dialog title="Manage agents" eyebrow="Settings · Agents" onClose={onClose} wide>
      <div className={styles.body}>
        {requests.data?.items.map((request) => (
          <ConnectionRequestBanner key={request.id} request={request} />
        ))}
        <AgentsTable
          onConnect={() => {
            setConnecting(true);
          }}
        />
      </div>
      {isConnecting && (
        <ConnectAgentDialog
          onClose={() => {
            setConnecting(false);
          }}
        />
      )}
    </Dialog>
  );
}
