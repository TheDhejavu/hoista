/**
 * Settings · Agents, in cards: coding tools on this computer, ChatGPT and
 * other web agents (MCP over HTTP, through a tunnel or your remote), your
 * remote, defaults for new agents, Hoista's page for approving them, and the
 * rules that always apply.
 */
import { Globe, Monitor, Plus, ShieldCheck, Terminal, Trash2 } from 'lucide-react';
import { useEffect, useState } from 'react';
import type { Settings } from '@shared/api';
import { useAgentAddress, useAgentLink, useForgetAgentLink } from '@ui/api/agents';
import { useSettings, useUpdateSettings } from '@ui/api/settings';
import { Button } from '@ui/components/Button';
import { CopyField } from '@ui/components/CopyField';
import { Dot } from '@ui/components/Dot';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { QueryState } from '@ui/components/QueryState';
import { RowMenu } from '@ui/components/RowMenu';
import {
  FixedValue,
  SelectRow,
  SettingGroup,
  SettingRow,
  SettingGroupSkeleton,
} from '@ui/components/SettingRow';
import { ACCESS_CHOICES } from '@ui/lib/agentAccess';
import { claudeCodeCommand, codexCommand, mcpCommand } from '@ui/lib/codingTools';
import { timeAgo } from '@ui/lib/format';
import { agentUrl, remoteName } from '@ui/lib/remote';
import { AgentsDialog } from '@ui/screens/agents/AgentsDialog';
import { DISCONNECT_CHOICES, ON_OFF_CHOICES } from './choices';
import { RemoteSettings } from './RemoteSettings';
import { OtherCodingTool } from './OtherCodingTool';
import { GROUP_ICON } from './groupIcon';
import styles from './AgentSettings.module.css';
import { SECOND_MS } from '@shared/time';
import { MCP_HTTP_PORT } from '@shared/ports';

/** How long MCP over HTTP may take to start listening before a busy port is reported. */
const LINK_STARTUP_MS = 6 * SECOND_MS;

/** Beside the section's title. The agents list opens over Settings, so Settings stays where it was. */
export function ManageAgentsButton() {
  const [managing, setManaging] = useState(false);
  return (
    <>
      <Button
        onClick={() => {
          setManaging(true);
        }}
      >
        Manage agents
      </Button>
      {managing && (
        <AgentsDialog
          onClose={() => {
            setManaging(false);
          }}
        />
      )}
    </>
  );
}

export function AgentSettings() {
  const settings = useSettings();
  return (
    <QueryState query={settings} skeleton={<SettingGroupSkeleton cards={2} />}>
      {(data) => <AgentCards settings={data} />}
    </QueryState>
  );
}

function AgentCards({ settings }: { settings: Settings }) {
  const update = useUpdateSettings();
  const address = useAgentAddress();
  const { agents } = settings;
  // The Hoista shown is on a server: agents connect at its own address.
  const shownServer = address.data?.public ?? null;

  return (
    <>
      {!shownServer && <CodingTools settings={settings} />}

      <WebAgents settings={settings} shownServer={shownServer} />

      <RemoteSettings remote={agents.remote} />

      <SettingGroup
        title="New agents"
        description="Hoista’s approval page starts on these; you can change each agent later."
        icon={<Plus {...GROUP_ICON} />}
      >
        <SelectRow
          label="What a new chat app can do"
          value={agents.default_access}
          options={ACCESS_CHOICES}
          onChange={(access) => {
            update.mutate({ agents: { default_access: access } });
          }}
        />
        <SelectRow
          label="Disconnect agents not used for"
          hint="Their access ends automatically. Reconnecting takes one approval."
          value={agents.disconnect_unused_after_days}
          options={DISCONNECT_CHOICES}
          onChange={(days) => {
            update.mutate({ agents: { disconnect_unused_after_days: days } });
          }}
        />
      </SettingGroup>

      {!shownServer && <HoistaPage settings={settings} />}

      <SettingGroup title="Security" icon={<ShieldCheck {...GROUP_ICON} />}>
        <SelectRow
          label="Agent connection"
          hint="Lets ChatGPT, Claude, Grok and coding tools reach Hoista."
          value={agents.connection_enabled}
          options={ON_OFF_CHOICES}
          onChange={(enabled) => {
            update.mutate({ agents: { connection_enabled: enabled } });
          }}
        />
        <SettingRow
          label="New chat apps connecting"
          hint="Each one must be approved on this computer before it can do anything."
        >
          <FixedValue>Always asks</FixedValue>
        </SettingRow>
      </SettingGroup>

      <SettingGroup
        title="Before deleting"
        icon={<Trash2 {...GROUP_ICON} />}
        note="With Ask for approval, the agent shows the question in its own window and you answer it there. Full access deletes without asking; View only can't delete."
      >
        <SettingRow label="Deleting a project, service, volume or domain">
          <FixedValue>Depends on each agent’s access</FixedValue>
        </SettingRow>
      </SettingGroup>
      <ErrorMessage error={update.error} />
    </>
  );
}

/** Coding tools start Hoista themselves, with the command Hoista found on this computer. */
function CodingTools({ settings }: { settings: Settings }) {
  const mcp = mcpCommand(settings.cli);
  return (
    <SettingGroup
      title="Coding tools"
      description="They start Hoista themselves on this computer."
      icon={<Terminal {...GROUP_ICON} />}
    >
      <SettingRow label="Claude Code">
        <CopyField text={claudeCodeCommand(mcp)} label="the Claude Code command" />
      </SettingRow>
      <SettingRow label="Codex">
        <CopyField text={codexCommand(mcp)} label="the Codex command" />
      </SettingRow>
      <OtherCodingTool cli={settings.cli} />
      <SettingRow
        label="Set up Claude Code and Codex"
        hint="Done for you when you connect them. To do it again, run this in Terminal."
      >
        <CopyField text="hoist skills install" label="the skills command" />
      </SettingRow>
    </SettingGroup>
  );
}

/**
 * ChatGPT and other web agents: MCP over HTTP through a tunnel or proxy to
 * this computer, or your remote's address, which works when this computer is off.
 */
function WebAgents({ settings, shownServer }: { settings: Settings; shownServer: string | null }) {
  const update = useUpdateSettings();
  const { agents } = settings;
  const { remote } = agents;
  const remoteMcp =
    remote.target !== 'this_computer' || remote.status !== 'off' ? agentUrl(agents) : null;

  return (
    <SettingGroup
      title="ChatGPT and web agents"
      description="Through a tunnel to this computer, or through your remote. You approve each one."
      icon={<Globe {...GROUP_ICON} />}
    >
      <SelectRow
        label="MCP over HTTP"
        hint={`On 127.0.0.1:${String(MCP_HTTP_PORT)}. Nothing outside this computer reaches it except through a tunnel or proxy you run.`}
        value={agents.mcp_http}
        options={ON_OFF_CHOICES}
        onChange={(enabled) => {
          update.mutate({ agents: { mcp_http: enabled } });
        }}
      />
      {shownServer ? (
        <SettingRow
          label="This Hoista"
          hint="Works from anywhere, for cloud agents, chat apps and coding tools. The first time, you approve the agent on its page with your passkey."
        >
          <CopyField text={`${shownServer}/mcp`} label="its address" />
        </SettingRow>
      ) : (
        agents.mcp_http && <ThroughThisComputer />
      )}
      {remoteMcp && (
        <SettingRow
          label="Through your remote"
          hint={
            remote.target === 'this_computer'
              ? 'Works when this computer is off'
              : `${remoteName(remote.target)} · works when this computer is off`
          }
        >
          <CopyField text={remoteMcp} label="your remote’s address" />
        </SettingRow>
      )}
    </SettingGroup>
  );
}

const THROUGH = 'Through this computer';

/** The address web agents use through your tunnel, learned from its first request. */
function ThroughThisComputer() {
  const link = useAgentLink();
  const forget = useForgetAgentLink();
  const data = link.data;
  // Just turned on, it takes a moment to listen: not a busy port yet.
  const [starting, setStarting] = useState(true);
  useEffect(() => {
    const done = setTimeout(() => {
      setStarting(false);
    }, LINK_STARTUP_MS);
    return () => {
      clearTimeout(done);
    };
  }, []);

  if (!data) {
    return (
      <SettingRow
        label={THROUGH}
        hint={link.isPending ? 'Reading the address' : 'Starts with the engine.'}
      />
    );
  }
  if (!data.listening && starting) {
    return <SettingRow label={THROUGH} hint="Starting MCP over HTTP" />;
  }
  if (!data.listening) {
    return (
      <SettingRow
        label={THROUGH}
        hint={`Port ${String(MCP_HTTP_PORT)} is in use by another app, so Hoista can’t serve MCP over HTTP. Close that app, or turn MCP over HTTP off.`}
      />
    );
  }
  if (!data.mcp) {
    return (
      <SettingRow
        label={THROUGH}
        hint="Point a tunnel or proxy at this computer, for example with this command, then add its https:// address followed by /mcp to the agent. The address shows here after the first request from outside."
      >
        <CopyField text={`cloudflared tunnel --url ${data.local}`} label="the tunnel command" />
      </SettingRow>
    );
  }
  return (
    <SettingRow
      label={THROUGH}
      hint={
        <span className={styles.status}>
          <Dot tone={forget.isPending || !data.seen_at ? 'neutral' : 'good'} small />
          {forget.isPending
            ? 'Forgetting address'
            : data.seen_at
              ? `Tunnel seen ${timeAgo(data.seen_at)}`
              : 'Waiting for a request through the tunnel'}
        </span>
      }
    >
      <span className={styles.addressControls}>
        <span className={styles.grow}>
          <CopyField text={data.mcp} label="the web agents’ address" />
        </span>
        <RowMenu
          label="More for this address"
          items={[
            {
              label: 'Forget address',
              danger: true,
              disabled: forget.isPending,
              onSelect: () => {
                forget.mutate();
              },
            },
          ]}
        />
      </span>
      {forget.error && <ErrorMessage error={forget.error} />}
    </SettingRow>
  );
}

/** Hoista's page, where you approve agents in a browser: here, and on your network when that's on. */
function HoistaPage({ settings }: { settings: Settings }) {
  const update = useUpdateSettings();
  const address = useAgentAddress();
  const local = address.data?.local ?? null;
  const network = address.data?.network ?? null;
  const networkOn = settings.agents.network_page;

  return (
    <SettingGroup
      title="Hoista’s page"
      description="Where you approve agents in a browser."
      icon={<Monitor {...GROUP_ICON} />}
    >
      {local ? (
        <SettingRow label="This computer" hint="Reachable only from this computer.">
          <CopyField text={local} label="this computer’s address" />
        </SettingRow>
      ) : (
        <SettingRow
          label="This computer"
          hint={address.isPending ? 'Reading the address' : 'Starts with the engine.'}
        />
      )}
      {networkOn &&
        (network ? (
          <SettingRow
            label="This network"
            hint="Opens Hoista’s page from other devices on the same Wi-Fi. It changes when you change networks."
          >
            <CopyField text={network} label="this network’s address" />
          </SettingRow>
        ) : (
          <SettingRow
            label="This network"
            hint="Waiting for a Wi-Fi or wired network. The address shows here once this computer is on one."
          />
        ))}
      <SelectRow
        label="Open on this network"
        hint="Lets other devices on your Wi-Fi open Hoista’s page to approve an agent."
        value={networkOn}
        options={ON_OFF_CHOICES}
        onChange={(enabled) => {
          update.mutate({ agents: { network_page: enabled } });
        }}
      />
    </SettingGroup>
  );
}
