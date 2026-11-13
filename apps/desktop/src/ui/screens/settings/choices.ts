/**
 * Dropdown choices for project-wide settings, worded as in the designs.
 */
import type { Settings } from '@shared/api';
import type { Option } from '@ui/components/Select';

export const THEME_CHOICES: Option<Settings['general']['theme']>[] = [
  { value: 'dark', label: 'Dark' },
  { value: 'light', label: 'Light' },
  { value: 'system', label: 'Match system' },
];

export const OPEN_AT_LOGIN_CHOICES: Option<Settings['general']['open_at_login']>[] = [
  { value: 'menu_bar', label: 'In the menu bar' },
  { value: 'window', label: 'As a window' },
  { value: 'off', label: "Don't open" },
];

export const NOTIFY_CHOICES: Option<Settings['general']['notify']>[] = [
  { value: 'deploys', label: 'Deploys and problems' },
  { value: 'problems', label: 'Only problems' },
  { value: 'never', label: 'Never' },
];

export const ON_OFF_CHOICES: Option<boolean>[] = [
  { value: true, label: 'On' },
  { value: false, label: 'Off' },
];

/** Days without use before an agent is disconnected: at most a year. */
export const DISCONNECT_CHOICES: Option<number | null>[] = [
  { value: 30, label: '30 days' },
  { value: 90, label: '90 days' },
  { value: 365, label: '1 year' },
];

export const CHANNEL_CHOICES: Option<Settings['updates']['channel']>[] = [
  { value: 'stable', label: 'Stable' },
  { value: 'beta', label: 'Beta' },
];

/** How often Hoista backs up what it knows. */
export const BACKUP_CHOICES: Option<Settings['backups']['every_hours']>[] = [
  { value: 6, label: 'Every 6 hours' },
  { value: 24, label: 'Every day' },
  { value: 168, label: 'Every week' },
  { value: 0, label: 'Off' },
];

/** How long activity and finished history are kept. */
export const HISTORY_CHOICES: Option<Settings['backups']['history_days']>[] = [
  { value: 30, label: '30 days' },
  { value: 90, label: '90 days' },
  { value: 365, label: 'A year' },
  { value: 1095, label: '3 years' },
];
