/**
 * How to add Hoista to one coding tool: its command, or the JSON for its
 * file of MCP servers with where that file is.
 */
import type { Settings } from '@shared/api';
import { CopyField } from '@ui/components/CopyField';
import { addHoista, type CodingTool } from '@ui/lib/codingTools';
import styles from './AddToTool.module.css';

export function AddToTool({ tool, cli }: { tool: CodingTool; cli: Settings['cli'] | undefined }) {
  const { text, lines, where } = addHoista(tool, cli);
  return (
    <div className={styles.add}>
      {where && <span className={styles.where}>{where}</span>}
      <CopyField
        text={text}
        lines={lines}
        label={lines ? `the ${tool.name} settings` : `the ${tool.name} command`}
      />
    </div>
  );
}
