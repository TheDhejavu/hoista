import { describe, expect, it } from 'vitest';
import { APP_COMMANDS, COMMAND_ROUTES, SHORTCUTS, commandForKey } from './shortcuts';

describe('keyboard shortcuts', () => {
  it('gives each key one command, and every command a place', () => {
    const keys = SHORTCUTS.map((shortcut) => shortcut.key);
    expect(new Set(keys).size).toBe(keys.length);
    for (const shortcut of SHORTCUTS) expect(APP_COMMANDS).toContain(shortcut.command);
    for (const command of Object.keys(COMMAND_ROUTES)) expect(APP_COMMANDS).toContain(command);
  });

  it('finds a command from the key, whatever its case', () => {
    expect(commandForKey('K')).toBe('palette');
    expect(commandForKey('2')).toBe('go-deploys');
    expect(commandForKey('[')).toBe('back');
    expect(commandForKey('q')).toBeUndefined();
  });
});
