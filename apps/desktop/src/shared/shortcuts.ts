/**
 * Hoista's keyboard shortcuts, in one place: the app menu shows them, the
 * window answers them, and the Keyboard shortcuts sheet lists them. ⌘ on
 * macOS, Ctrl on Linux and Windows.
 */

/** Something a shortcut or a menu item does in the main window. */
export type AppCommand =
  | 'palette'
  | 'new-deploy'
  | 'settings'
  | 'go-projects'
  | 'go-deploys'
  | 'go-agents'
  | 'go-accounts'
  | 'go-billing'
  | 'back'
  | 'forward'
  | 'refresh'
  | 'shortcuts'
  | 'help';

export interface Shortcut {
  command: AppCommand;
  label: string;
  /** The key pressed with ⌘ (Ctrl), as `KeyboardEvent.key` gives it, lowercase. */
  key: string;
  group: 'General' | 'Go to' | 'View';
}

export const SHORTCUTS: readonly Shortcut[] = [
  { command: 'palette', label: 'Search or jump to', key: 'k', group: 'General' },
  { command: 'new-deploy', label: 'Deploy something', key: 'n', group: 'General' },
  { command: 'settings', label: 'Settings', key: ',', group: 'General' },
  { command: 'shortcuts', label: 'Keyboard shortcuts', key: '/', group: 'General' },
  { command: 'go-projects', label: 'Projects', key: '1', group: 'Go to' },
  { command: 'go-deploys', label: 'Deploys', key: '2', group: 'Go to' },
  { command: 'go-agents', label: 'Agents', key: '3', group: 'Go to' },
  { command: 'go-accounts', label: 'Accounts', key: '4', group: 'Go to' },
  { command: 'go-billing', label: 'Billing', key: '5', group: 'Go to' },
  { command: 'back', label: 'Back', key: '[', group: 'Go to' },
  { command: 'forward', label: 'Forward', key: ']', group: 'Go to' },
  { command: 'refresh', label: 'Refresh', key: 'r', group: 'View' },
];

/** Every command, for checking what the menus send. */
export const APP_COMMANDS: readonly AppCommand[] = [
  'palette',
  'new-deploy',
  'settings',
  'go-projects',
  'go-deploys',
  'go-agents',
  'go-accounts',
  'go-billing',
  'back',
  'forward',
  'refresh',
  'shortcuts',
  'help',
];

/** The routes the Go to commands open. */
export const COMMAND_ROUTES: Partial<Record<AppCommand, string>> = {
  'new-deploy': '/deploys',
  'go-projects': '/',
  'go-deploys': '/deploys',
  'go-agents': '/agents',
  'go-accounts': '/accounts',
  'go-billing': '/billing',
};

/** The shortcut for a command, if it has one. */
export function shortcutFor(command: AppCommand): Shortcut | undefined {
  return SHORTCUTS.find((shortcut) => shortcut.command === command);
}

/** The command a key pressed with ⌘ (Ctrl) runs, if any. */
export function commandForKey(key: string): AppCommand | undefined {
  return SHORTCUTS.find((shortcut) => shortcut.key === key.toLowerCase())?.command;
}

/** How a target sent from a menu names a command: `cmd:refresh`. */
export const COMMAND_PREFIX = 'cmd:';
