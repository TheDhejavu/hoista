/**
 * The coding tools Hoista can be added to, and how. Built from the MCP
 * command Hoista found on this computer, which has the full path when
 * `hoist` isn't on PATH. Some tools add it with their own command; the rest
 * read a JSON file of MCP servers.
 */
import type { Settings } from '@shared/api';

/** What a coding tool runs to start Hoista, ready to paste. */
export function mcpCommand(cli: Settings['cli'] | undefined): string {
  return cli?.mcp_command ?? 'hoist mcp';
}

/** The same command as its words, for JSON files. */
export function mcpArgs(cli: Settings['cli'] | undefined): string[] {
  return cli?.mcp_args ?? ['hoist', 'mcp'];
}

export function claudeCodeCommand(mcp: string): string {
  return `claude mcp add --scope user hoista -- ${mcp}`;
}

export function codexCommand(mcp: string): string {
  return `codex mcp add hoista -- ${mcp}`;
}

/** One stdio server entry, the shape most tools read. */
function server(args: string[], extra: Record<string, unknown> = {}) {
  const [command = 'hoist', ...rest] = args;
  return { ...extra, command, args: rest };
}

/** Single quotes for a shell, so the JSON reaches the command as one word. */
function quoted(text: string): string {
  return `'${text.replace(/'/g, `'\\''`)}'`;
}

/** A file of MCP servers with Hoista in it, the way most tools read it. */
function mcpServers(args: string[]): string {
  const text = JSON.stringify({ mcpServers: { hoista: server(args) } }, null, 2);
  // The arguments read better on one line: "args": ["mcp", "--state-dir", "…"].
  return text.replace(
    /\[\n\s*([^\]]*?)\n\s*\]/g,
    (_all, items: string) => `[${items.split(/,\n\s*/).join(', ')}]`,
  );
}

export type AddHoista =
  /** Run the tool's own command; it gets the pasteable command and its words. */
  | { kind: 'command'; command: (mcp: string, args: string[]) => string }
  /** Add an entry to the tool's JSON file of MCP servers; `where` names it. */
  | { kind: 'file'; where: string; config: (args: string[]) => string };

export interface CodingTool {
  id: string;
  name: string;
  add: AddHoista;
}

export const CLAUDE_CODE: CodingTool = {
  id: 'claude-code',
  name: 'Claude Code',
  add: { kind: 'command', command: claudeCodeCommand },
};

export const CODEX: CodingTool = {
  id: 'codex',
  name: 'Codex',
  add: { kind: 'command', command: codexCommand },
};

/** Tools set up by hand, in the order they're offered. */
export const OTHER_CODING_TOOLS: CodingTool[] = [
  {
    id: 'cursor',
    name: 'Cursor',
    add: { kind: 'file', where: 'Add this to ~/.cursor/mcp.json', config: mcpServers },
  },
  {
    id: 'vscode',
    name: 'VS Code (Copilot)',
    add: {
      kind: 'command',
      command: (_mcp, args) =>
        `code --add-mcp ${quoted(JSON.stringify({ name: 'hoista', ...server(args, { type: 'stdio' }) }))}`,
    },
  },
  {
    id: 'windsurf',
    name: 'Windsurf',
    add: {
      kind: 'file',
      where: 'Add this to ~/.codeium/windsurf/mcp_config.json',
      config: mcpServers,
    },
  },
  {
    id: 'gemini-cli',
    name: 'Gemini CLI',
    add: { kind: 'file', where: 'Add this to ~/.gemini/settings.json', config: mcpServers },
  },
  {
    id: 'cline',
    name: 'Cline',
    add: {
      kind: 'file',
      where: 'In Cline, open MCP Servers, then Configure MCP Servers, and add this',
      config: mcpServers,
    },
  },
];

/** Anything else that runs MCP servers: the usual file shape. */
export const ANY_MCP_TOOL: CodingTool = {
  id: 'other-mcp',
  name: 'Another tool that supports MCP',
  add: { kind: 'file', where: 'Add this where the tool keeps its MCP servers', config: mcpServers },
};

/** What to copy for a tool, whether it spans lines, and where it goes. */
export function addHoista(
  tool: CodingTool,
  cli: Settings['cli'] | undefined,
): { text: string; lines: boolean; where: string | null } {
  const { add } = tool;
  if (add.kind === 'command') {
    return { text: add.command(mcpCommand(cli), mcpArgs(cli)), lines: false, where: null };
  }
  return {
    text: add.config(mcpArgs(cli)),
    lines: true,
    where: `${add.where}. If the file already lists MCP servers, put the hoista entry beside them.`,
  };
}
