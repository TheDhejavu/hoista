/**
 * Setup step 3: where you talk to AI, and how to add Hoista there. Coding
 * tools get the command that adds Hoista, right here. Chat apps and cloud
 * agents reach Hoista at its address, which the Connect an agent window sets
 * up (a tunnel on this computer, or a remote). A starter prompt shows the
 * agent what Hoista can do.
 */
import { useId, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { FileText } from 'lucide-react';
import type { Settings } from '@shared/api';
import { useSettings } from '@ui/api/settings';
import { AgentLogo } from '@ui/components/AgentIcon';
import { CopyPromptButton } from '@ui/components/AgentPrompt';
import { Button } from '@ui/components/Button';
import { QueryState } from '@ui/components/QueryState';
import { Select, type Option } from '@ui/components/Select';
import { Step, Steps } from '@ui/components/Steps';
import { AddToTool } from '@ui/screens/agents/AddToTool';
import { CLIENTS, type Client } from '@ui/screens/agents/clients';
import { ConnectAgentDialog } from '@ui/screens/agents/ConnectAgentDialog';
import { AGENT_PROMPT } from '@ui/screens/projects/GetStarted';
import { SetupStep } from './SetupStep';
import { stepPath, usePicked, useSetup } from './useSetup';
import styles from './Setup.module.css';

const CLIENT_CHOICES: Option<Client | null>[] = [
  { value: null, label: 'None' },
  ...CLIENTS.map((client) => ({ value: client, label: client.name, group: client.group })),
];

const FIRST_QUESTION = 'what projects do I have on Hoista?';

export function ConnectAIStep() {
  const navigate = useNavigate();
  const picked = usePicked();
  const setup = useSetup();
  const id = useId();
  // Nothing picked until you choose: the steps depend on where you talk to AI.
  const [client, setClient] = useState<Client | null>(null);
  const settings = useSettings();

  return (
    <SetupStep
      number={3}
      title="Connect an agent"
      lede="Your agent can then check status, read logs and bills, and deploy through Hoista. You choose what it may do."
      narrow
      back={
        <Button
          size="xl"
          onClick={() => {
            void navigate(stepPath('projects', picked));
          }}
        >
          Back
        </Button>
      }
      error={setup.finishError}
      actions={
        <>
          <Button variant="ghost" size="xl" disabled={setup.finishing} onClick={setup.skip}>
            Skip for now
          </Button>
          <Button
            variant="accent"
            size="xl"
            busy={setup.finishing}
            busyLabel="Finishing"
            onClick={() => {
              setup.finish(client?.name ?? null);
            }}
          >
            Finish
          </Button>
        </>
      }
    >
      <div className={styles.agentPick}>
        <label htmlFor={id} className={styles.fieldLabel}>
          Which agent do you use?
        </label>
        <Select
          id={id}
          large
          wide
          value={client}
          options={CLIENT_CHOICES}
          onChange={setClient}
          icon={client ? <AgentLogo name={client.name} size={18} /> : undefined}
        />
      </div>
      {client ? (
        <QueryState query={settings}>
          {(data) => <AgentSteps client={client} settings={data} />}
        </QueryState>
      ) : (
        <p className={styles.pickHint}>Pick one to see how to connect it.</p>
      )}
      <div className={styles.starter}>
        <FileText size={18} aria-hidden="true" />
        <span>
          Not sure what to ask? Copy a starter prompt that shows your agent what Hoista can do.
        </span>
        <CopyPromptButton icon={false} label="Copy starter prompt" prompt={AGENT_PROMPT} />
      </div>
    </SetupStep>
  );
}

/** The numbered steps for one tool. */
function AgentSteps({ client, settings }: { client: Client; settings: Settings }) {
  const { connect } = client;
  const [settingUp, setSettingUp] = useState(false);

  return (
    <div className={styles.agentSteps}>
      <Steps>
        {connect.kind === 'link' ? (
          <>
            <Step number={1} title={`Give ${client.name} Hoista’s address`}>
              <span className={styles.stepHint}>
                {client.name} reaches Hoista through a tunnel on this computer, or a remote that
                stays on when it’s off.
              </span>
              <span>
                <Button
                  large
                  onClick={() => {
                    setSettingUp(true);
                  }}
                >
                  Get the address
                </Button>
              </span>
              {settingUp && (
                <ConnectAgentDialog
                  initial={client}
                  onClose={() => {
                    setSettingUp(false);
                  }}
                />
              )}
            </Step>
            <Step number={2} title={`Add it in ${client.name}`}>
              <span className={styles.stepHint}>{connect.steps}</span>
            </Step>
            <Step number={3} title={`Ask ${client.name}: “${FIRST_QUESTION}”`}>
              <span className={styles.stepHint}>The first time, Hoista asks you to allow it.</span>
            </Step>
          </>
        ) : (
          <>
            <Step number={1} title={`Add Hoista to ${client.name}`}>
              <AddToTool tool={connect.tool} cli={settings.cli} />
            </Step>
            <Step number={2} title={`Ask ${client.name}: “${FIRST_QUESTION}”`}>
              <span className={styles.stepHint}>The first time, Hoista asks you to allow it.</span>
            </Step>
          </>
        )}
      </Steps>
    </div>
  );
}
