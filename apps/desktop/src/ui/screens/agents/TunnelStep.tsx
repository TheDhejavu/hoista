import { useEffect, useState } from 'react';
import { LoaderCircle } from 'lucide-react';
import { useAgentLink, useSetAgentLink } from '@ui/api/agents';
import { Button } from '@ui/components/Button';
import { CopyField } from '@ui/components/CopyField';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { Step, Steps } from '@ui/components/Steps';
import styles from './TunnelStep.module.css';
import shared from './connectAgent.module.css';
import { MCP_HTTP_ADDRESS } from '@shared/ports';
import { PortBusy } from './DoneStep';

export function TunnelStep({
  clientName,
  onBack,
  onClose,
  onAddress,
}: {
  clientName: string;
  onBack: () => void;
  onClose: () => void;
  onAddress: () => void;
}) {
  const link = useAgentLink();
  const setLink = useSetAgentLink();
  const [address, setAddress] = useState('');
  const known = link.data?.mcp ?? null;
  // The address Hoista knew when this step opened, so coming back here to
  // use a different tunnel doesn't bounce straight on.
  const [before] = useState(known);

  // A request through a new tunnel teaches Hoista its address: carry on.
  useEffect(() => {
    if (known && known !== before) onAddress();
  }, [known, before, onAddress]);

  return (
    <Dialog
      depth={1}
      title={`Connect ${clientName}`}
      eyebrow="Through this computer"
      onBack={onBack}
      backLabel={`Connect ${clientName}`}
      onClose={onClose}
      actions={
        <Button large onClick={onClose}>
          Cancel
        </Button>
      }
    >
      {link.data && !link.data.listening && <PortBusy />}
      <Steps>
        <Step number={1} title="Start a tunnel to Hoista">
          <CopyField
            text={`cloudflared tunnel --url ${link.data?.local ?? MCP_HTTP_ADDRESS}`}
            label="the tunnel command"
          />
          <span className={shared.hint}>
            Run it in Terminal. Any HTTPS tunnel or proxy works too, such as ngrok, Tailscale Funnel
            or Caddy.
          </span>
        </Step>
        <Step number={2} title="Paste the address it prints">
          <form
            className={styles.paste}
            onSubmit={(event) => {
              event.preventDefault();
              setLink.mutate(address.trim(), { onSuccess: onAddress });
            }}
          >
            <input
              className={styles.input}
              aria-label="Tunnel address"
              placeholder="https://….trycloudflare.com"
              value={address}
              onChange={(event) => {
                setAddress(event.target.value);
              }}
            />
            <Button
              type="submit"
              variant="accent"
              busy={setLink.isPending}
              busyLabel="Checking"
              disabled={!address.trim()}
            >
              Use it
            </Button>
          </form>
          <span className={styles.waiting}>
            <LoaderCircle size={14} className={shared.spin} aria-hidden="true" />
            Or open it in a browser once and Hoista picks it up by itself.
          </span>
          <ErrorMessage error={setLink.error} />
        </Step>
      </Steps>
    </Dialog>
  );
}
