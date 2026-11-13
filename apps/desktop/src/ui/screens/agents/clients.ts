/**
 * The AI tools Hoista can connect to, and how each one connects.
 *
 * Chat apps and cloud agents reach Hoista at its address: MCP over HTTP on
 * this computer through a tunnel, or a remote's Hoista URL. Coding tools
 * start `hoist mcp` themselves, added with their own command or a JSON
 * file of MCP servers (see lib/codingTools). Cloud agents such as Hermes
 * run on their own schedule and keep an eye on your projects between sessions.
 */
import {
  ANY_MCP_TOOL,
  CLAUDE_CODE,
  CODEX,
  OTHER_CODING_TOOLS,
  type CodingTool,
} from '@ui/lib/codingTools';

type HowToConnect =
  /** Paste the Hoista URL into the tool; `steps` says where. */
  | { kind: 'link'; steps: string }
  /** A coding tool that starts `hoist mcp` itself. */
  | { kind: 'tool'; tool: CodingTool };

export interface Client {
  id: string;
  name: string;
  group: 'Chat apps' | 'Coding tools' | 'Cloud agents' | 'Anything else';
  connect: HowToConnect;
}

const CHATGPT: Client = {
  id: 'chatgpt',
  name: 'ChatGPT',
  group: 'Chat apps',
  connect: {
    kind: 'link',
    steps:
      'In ChatGPT, turn on Developer mode in Settings → Security and login. Then open Plugins → Add → Create MCP App, name it "Hoista", paste Hoista’s address and choose OAuth. Needs a paid plan.',
  },
};

export const CLIENTS: Client[] = [
  CHATGPT,
  {
    id: 'claude',
    name: 'Claude',
    group: 'Chat apps',
    connect: {
      kind: 'link',
      steps:
        'In Claude, open Customize → Connectors → Add custom connector. Name it "Hoista" and paste Hoista’s address.',
    },
  },
  {
    id: 'grok',
    name: 'Grok',
    group: 'Chat apps',
    connect: {
      kind: 'link',
      steps:
        'Go to grok.com/connectors, choose New Connector → Custom, and paste Hoista’s address.',
    },
  },
  {
    id: 'gemini',
    name: 'Gemini',
    group: 'Chat apps',
    connect: {
      kind: 'link',
      steps:
        'In Gemini on the web, open Settings → Connected Apps → Add a custom app and paste Hoista’s address. US only, with a personal Google account.',
    },
  },
  ...[CLAUDE_CODE, CODEX, ...OTHER_CODING_TOOLS].map<Client>((tool) => ({
    id: tool.id,
    name: tool.name,
    group: 'Coding tools',
    connect: { kind: 'tool', tool },
  })),
  {
    id: 'hermes',
    name: 'Hermes',
    group: 'Cloud agents',
    connect: {
      kind: 'link',
      steps:
        'In Hermes, add an MCP server named "hoista" under mcp_servers in config.yaml, with your Hoista URL and auth: oauth. (Hermes running on this computer can use the command instead.) Then give it a daily job, for example: "Each morning, ask Hoista what needs attention, fix what you can, and message me about the rest."',
    },
  },
  {
    id: 'other',
    name: 'Another app that supports MCP',
    group: 'Anything else',
    connect: { kind: 'tool', tool: ANY_MCP_TOOL },
  },
];
