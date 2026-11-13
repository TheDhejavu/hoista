import { Check, ShieldCheck, TriangleAlert } from 'lucide-react';
import type { AccessLevel, Settings } from '@shared/api';
import { Button } from '@ui/components/Button';
import { CopyField } from '@ui/components/CopyField';
import { Dialog } from '@ui/components/Dialog';
import { Step, Steps } from '@ui/components/Steps';
import { ACCESS_CHOICES } from '@ui/lib/agentAccess';
import { agentUrl } from '@ui/lib/remote';
import { AddToTool } from './AddToTool';
import { type Client } from './clients';
import styles from './DoneStep.module.css';
import shared from './connectAgent.module.css';
import { MCP_HTTP_PORT } from '@shared/ports';
import { FIRST_QUESTION } from './ConnectAgentDialog';

export function DoneStep({
  clientName,
  access,
  everyProject,
  onClose,
}: {
  clientName: string;
  access: AccessLevel;
  everyProject: boolean;
  onClose: () => void;
}) {
  const can = ACCESS_CHOICES.find((choice) => choice.value === access)?.label ?? '';
  return (
    <Dialog
      depth={4}
      title={`${clientName} is connected`}
      onClose={onClose}
      actions={
        <Button variant="accent" large onClick={onClose}>
          Done
        </Button>
      }
    >
      <div className={styles.done}>
        <span className={styles.doneTick} aria-hidden="true">
          <Check size={26} />
        </span>
        <p>
          Set to {can}, on {everyProject ? 'every project' : 'the project you picked'}. It works
          while this computer is awake and your tunnel runs. Change it any time on the Agents page.
        </p>
        <div className={styles.ask}>
          <span className={styles.askArrow} aria-hidden="true">
            →
          </span>
          Ask {clientName}: “{FIRST_QUESTION}”
        </div>
      </div>
    </Dialog>
  );
}

/** A remote already running: its Hoista URL, pasted into the app. */
export function RemoteSteps({ client, settings }: { client: Client; settings: Settings }) {
  if (client.connect.kind !== 'link') return null;
  const onServer = Boolean(settings.agents.remote.url);
  return (
    <>
      <Steps>
        <Step number={1} title="Copy your Hoista URL">
          <CopyField text={agentUrl(settings.agents) ?? ''} label="Hoista URL" />
        </Step>
        <Step number={2} title={`Add it in ${client.name}`}>
          <span className={shared.hint}>{client.connect.steps}</span>
        </Step>
        <Step
          number={3}
          title={
            onServer
              ? 'Sign in with your passkey when it asks'
              : 'Approve the sign-in Hoista shows you'
          }
        >
          <span className={shared.hint}>
            {onServer
              ? `${client.name} opens your Hoista’s page. Sign in there and choose what it can do. Nobody else can connect without your passkey.`
              : `This proves it’s your ${client.name}. Nobody else can use the URL.`}
          </span>
        </Step>
        <Step number={4} title={`Ask ${client.name}: "${FIRST_QUESTION}"`} />
      </Steps>
      <span className={styles.note}>
        <ShieldCheck size={15} aria-hidden="true" />
        You can change what each agent can do any time on the Agents page.
      </span>
    </>
  );
}

/** A coding tool: the command that adds Hoista, then a first question. */
export function CodingToolSteps({ client, settings }: { client: Client; settings: Settings }) {
  if (client.connect.kind !== 'tool') return null;
  return (
    <Steps>
      <Step number={1} title={`Add Hoista to ${client.name}`}>
        <AddToTool tool={client.connect.tool} cli={settings.cli} />
      </Step>
      <Step number={2} title={`Ask ${client.name}: "${FIRST_QUESTION}"`}>
        <span className={shared.hint}>
          Coding tools start Hoista themselves, so they work even when this window is closed.
        </span>
      </Step>
    </Steps>
  );
}

/**
 * Hoista isn't listening on its port: something else has it, often another
 * Hoista started with a different state folder. A tunnel to the port then
 * reaches that one, so agents connect there and never show up here.
 */
export function PortBusy() {
  return (
    <p className={styles.portBusy} role="alert">
      <TriangleAlert size={16} aria-hidden="true" />
      <span>
        Port {MCP_HTTP_PORT} is in use by another program, often another Hoista. Your tunnel reaches
        that one, not this one, so agents that connect never show up here. Stop it, and this Hoista
        takes the port by itself within seconds.
      </span>
    </p>
  );
}
