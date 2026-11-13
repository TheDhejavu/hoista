import { describe, expect, it } from 'vitest';
import { agentDisplayName, agentLogo } from './agentLogos';

describe('agent names and logos', () => {
  it('shows the tool an MCP client name belongs to, with its logo', () => {
    for (const [registered, shown] of [
      ['codex-mcp-client', 'Codex'],
      ['Codex Mcp Client', 'Codex'],
      ['claude-code', 'Claude Code'],
      ['cursor-vscode', 'Cursor'],
      ['gemini-cli-mcp-client', 'Gemini'],
      ['windsurf-client', 'Windsurf'],
    ] as const) {
      expect(agentDisplayName(registered)).toBe(shown);
      expect(agentLogo(registered)).not.toBeNull();
    }
  });

  it('title-cases names it does not know and gives them no logo', () => {
    expect(agentDisplayName('my-deploy-bot')).toBe('My Deploy Bot');
    expect(agentLogo('my-deploy-bot')).toBeNull();
  });
});
