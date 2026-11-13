/**
 * Coding tools besides Claude Code and Codex: pick one to see how to add
 * Hoista to it, its command or the JSON for its file of MCP servers.
 */
import { useState } from 'react';
import type { Settings } from '@shared/api';
import { AgentIcon } from '@ui/components/AgentIcon';
import { SelectRow, SettingRow } from '@ui/components/SettingRow';
import { ANY_MCP_TOOL, OTHER_CODING_TOOLS, type CodingTool } from '@ui/lib/codingTools';
import { AddToTool } from '@ui/screens/agents/AddToTool';
import styles from './OtherCodingTool.module.css';

const TOOLS: CodingTool[] = [...OTHER_CODING_TOOLS, ANY_MCP_TOOL];

export function OtherCodingTool({ cli }: { cli: Settings['cli'] }) {
  const [picked, setPicked] = useState<CodingTool | null>(null);
  return (
    <>
      <SelectRow<CodingTool | null>
        label="Another coding tool"
        hint="Cursor, VS Code, Windsurf, Gemini CLI, Cline, or anything that runs MCP servers."
        value={picked}
        options={[
          { value: null, label: 'None' },
          ...TOOLS.map((tool) => ({ value: tool, label: tool.name })),
        ]}
        isEqual={(a, b) => a?.id === b?.id}
        onChange={setPicked}
      />
      {picked && (
        <SettingRow
          label={`Add Hoista to ${picked.name}`}
          icon={<AgentIcon name={picked.name} small />}
        >
          <div className={styles.full}>
            <AddToTool tool={picked} cli={cli} />
          </div>
        </SettingRow>
      )}
    </>
  );
}
