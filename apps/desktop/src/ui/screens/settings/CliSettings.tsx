/**
 * Settings · Command line: the `hoist` command Hoista uses, and the exact
 * command each coding tool needs to reach this Hoista over MCP. The commands
 * come from the engine, so they point at the real binary and state folder
 * even when `hoist` isn't on the PATH.
 */
import type { Settings } from '@shared/api';
import { useSettings } from '@ui/api/settings';
import { AgentIcon } from '@ui/components/AgentIcon';
import { CopyField } from '@ui/components/CopyField';
import { QueryState } from '@ui/components/QueryState';
import { SettingGroup, SettingRow, SettingGroupSkeleton } from '@ui/components/SettingRow';
import { claudeCodeCommand, codexCommand } from '@ui/lib/codingTools';
import { OtherCodingTool } from './OtherCodingTool';
import { SectionHeader } from './SectionHeader';
import styles from './CliSettings.module.css';

/** Commands that exist in `hoist` today, with what they do. */
const USEFUL_COMMANDS = [
  { command: 'hoist projects', does: 'list your projects and their status' },
  { command: 'hoist deploy', does: 'deploy the folder you’re in' },
  { command: 'hoist logs <project> -f', does: 'follow a project’s logs' },
  { command: 'hoist import', does: 'bring in what already runs on your providers' },
  { command: 'hoist mcp config', does: 'print the MCP settings for any tool' },
];

export function CliSettings() {
  const settings = useSettings();

  return (
    <>
      <SectionHeader title="Command line" />
      <QueryState query={settings} skeleton={<SettingGroupSkeleton cards={2} />}>
        {(data) => <CliGroups cli={data.cli} />}
      </QueryState>
    </>
  );
}

function CliGroups({ cli }: { cli: Settings['cli'] }) {
  const mcp = cli.mcp_command;
  const found = mcp !== null;

  return (
    <>
      <SettingGroup title="The hoist command">
        <SettingRow
          label={
            found
              ? `Installed${cli.version ? ` · v${cli.version}` : ''}`
              : 'Not found on this computer'
          }
          hint={
            found ? (
              <>
                <code className={styles.path}>{cli.path}</code>
                {!cli.installed && (
                  <span className={styles.pathNote}>
                    Not on your PATH, so the commands below use its full path.
                  </span>
                )}
              </>
            ) : (
              'Hoista couldn’t find its hoist binary, so coding tools can’t start it yet.'
            )
          }
        />
      </SettingGroup>

      {found && (
        <SettingGroup
          title="Connect a coding tool"
          description="Coding tools start Hoista themselves, so they work even when this window is closed."
        >
          <ToolRow name="Claude Code" command={claudeCodeCommand(mcp)} />
          <ToolRow name="Codex" command={codexCommand(mcp)} />
          <OtherCodingTool cli={cli} />
        </SettingGroup>
      )}

      <SettingGroup title="Useful commands">
        <dl className={styles.commands}>
          {USEFUL_COMMANDS.map((item) => (
            <div key={item.command} className={styles.command}>
              <dt>
                <code>{item.command}</code>
              </dt>
              <dd>{item.does}</dd>
            </div>
          ))}
        </dl>
      </SettingGroup>
    </>
  );
}

function ToolRow({
  name,
  label = name,
  hint,
  command,
}: {
  name: string;
  label?: string;
  hint?: string;
  command: string;
}) {
  return (
    <SettingRow icon={<AgentIcon name={name} small />} label={label} hint={hint}>
      <div className={styles.copy}>
        <CopyField text={command} label={`${label} command`} />
      </div>
    </SettingRow>
  );
}
