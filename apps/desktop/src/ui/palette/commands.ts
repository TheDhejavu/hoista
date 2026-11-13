/**
 * The commands the palette offers, and how a search narrows them.
 *
 * A command goes somewhere (a screen, or the Settings or Get help dialog),
 * opens the project's site, or copies its address. Changes to projects are
 * made where they're shown, with their own confirmations, so the palette
 * takes you there rather than running one itself. Each command also answers
 * to the words people use for it ("redeploy", "env", "dns").
 */
import type { ProjectSummary } from '@shared/api';
import { shortcutFor, type AppCommand } from '@shared/shortcuts';

export type Command = {
  id: string;
  group: 'Actions' | 'Go to' | 'This project';
  label: string;
  /** Other words it answers to, such as `rollback` for Deploys. */
  keywords?: string[];
  /** The key pressed with ⌘ (Ctrl) that does the same, shown beside it. */
  shortcut?: string;
} & (
  | {
      /** Route to open, for example `/projects/moodboard/logs`. */
      to: string;
    }
  | {
      /** Dialog to open over the current screen. */
      opens: 'settings' | 'help' | 'shortcuts';
    }
  | {
      /** A web address to open in the browser. */
      external: string;
    }
  | {
      /** Text to copy. */
      copy: string;
      /** What was copied, for the confirmation: "Address", "Command". */
      copied: string;
    }
);

const PAGES: Command[] = [
  { id: 'go-projects', group: 'Go to', label: 'Projects', to: '/', shortcut: key('go-projects') },
  {
    id: 'go-deploys',
    group: 'Go to',
    label: 'Deploys',
    to: '/deploys',
    keywords: ['releases'],
    shortcut: key('go-deploys'),
  },
  { id: 'go-agents', group: 'Go to', label: 'Agents', to: '/agents', shortcut: key('go-agents') },
  {
    id: 'go-accounts',
    group: 'Go to',
    label: 'Accounts',
    to: '/accounts',
    keywords: ['providers', 'tokens', 'replace token', 'connect a provider'],
    shortcut: key('go-accounts'),
  },
  {
    id: 'go-billing',
    group: 'Go to',
    label: 'Billing',
    to: '/billing',
    keywords: ['usage', 'spend', 'cost', 'invoice', 'limits', 'plan'],
    shortcut: key('go-billing'),
  },
  {
    id: 'go-settings',
    group: 'Go to',
    label: 'Settings',
    opens: 'settings',
    keywords: ['preferences'],
    shortcut: key('settings'),
  },
  {
    id: 'go-help',
    group: 'Go to',
    label: 'Get help',
    opens: 'help',
    keywords: ['support', 'docs'],
  },
  {
    id: 'go-shortcuts',
    group: 'Go to',
    label: 'Keyboard shortcuts',
    opens: 'shortcuts',
    keywords: ['keys', 'hotkeys', 'keyboard'],
    shortcut: key('shortcuts'),
  },
];

/** A project's tabs, and the words people use for what's on them. */
const TABS: { tab: string; label: string; keywords: string[] }[] = [
  {
    tab: 'deploys',
    label: 'Deploys',
    keywords: ['redeploy', 'rollback', 'roll back', 'versions', 'releases'],
  },
  { tab: 'logs', label: 'Logs', keywords: ['errors', 'output'] },
  { tab: 'metrics', label: 'Metrics', keywords: ['requests', 'cpu', 'memory', 'traffic'] },
  {
    tab: 'env',
    label: 'Variables',
    keywords: ['env', 'environment variables', 'secrets', 'add variable'],
  },
  { tab: 'domains', label: 'Domains', keywords: ['dns', 'custom domain', 'ssl', 'certificate'] },
  { tab: 'provider', label: 'Providers', keywords: ['operations', 'restart', 'scale', 'console'] },
  { tab: 'map', label: 'Map', keywords: ['architecture', 'connections'] },
  { tab: 'settings', label: 'Settings', keywords: ['rename', 'delete', 'source', 'environment'] },
];

/** One project's commands: its overview, each tab, and its site. */
function projectCommands(project: ProjectSummary, group: Command['group']): Command[] {
  const base = `/projects/${project.id}`;
  const commands: Command[] = [
    { id: `go-${project.id}`, group, label: project.name, to: base },
    ...TABS.map((tab) => ({
      id: `${tab.tab}-${project.id}`,
      group,
      label: `${project.name} · ${tab.label}`,
      to: `${base}/${tab.tab}`,
      keywords: tab.keywords,
    })),
  ];
  if (project.url) {
    commands.push(
      {
        id: `open-${project.id}`,
        group,
        label: `Open ${project.name}’s site`,
        external: project.url,
        keywords: ['visit', 'website', 'url'],
      },
      {
        id: `copy-${project.id}`,
        group,
        label: `Copy ${project.name}’s address`,
        copy: project.url,
        copied: 'Address',
        keywords: ['url', 'link'],
      },
    );
  }
  return commands;
}

/** What a developer copies for the open project: its CLI commands and its id. */
function developerCommands(project: ProjectSummary): Command[] {
  const name = /^[a-z0-9-]+$/.test(project.name) ? project.name : project.id;
  return [
    {
      id: `cli-logs-${project.id}`,
      group: 'This project',
      label: `Copy the command to follow ${project.name}’s logs`,
      copy: `hoist logs ${name} --follow`,
      copied: 'Command',
      keywords: ['cli', 'terminal', 'tail', 'hoist'],
    },
    {
      id: `cli-status-${project.id}`,
      group: 'This project',
      label: `Copy the command to see ${project.name}’s status`,
      copy: `hoist status ${name}`,
      copied: 'Command',
      keywords: ['cli', 'terminal', 'hoist'],
    },
    {
      id: `copy-id-${project.id}`,
      group: 'This project',
      label: `Copy ${project.name}’s project ID`,
      copy: project.id,
      copied: 'Project ID',
      keywords: ['id', 'uuid', 'mcp', 'agent'],
    },
  ];
}

/** The key for a command that has one. */
function key(command: AppCommand): string | undefined {
  return shortcutFor(command)?.key;
}

/** All commands for the current list of projects; the one open now comes first. */
export function buildCommands(projects: ProjectSummary[], current?: string): Command[] {
  const actions: Command[] = [
    { id: 'deploy', group: 'Actions', label: 'Deploy with your agent', to: '/deploys' },
    { id: 'connect-agent', group: 'Actions', label: 'Connect an agent', to: '/agents?connect=1' },
  ];
  const here = projects.find((project) => project.id === current);
  const others = projects.filter((project) => project !== here);
  return [
    ...(here ? [...projectCommands(here, 'This project'), ...developerCommands(here)] : []),
    ...actions,
    ...others.flatMap((project) => projectCommands(project, 'Go to')),
    ...PAGES,
  ];
}

/** Commands whose label contains the search, ignoring case. An empty search keeps the first few. */
export function filterCommands(commands: Command[], search: string, limit = 8): Command[] {
  const text = search.trim().toLowerCase();
  if (!text) return commands.slice(0, limit);
  // Names first, then what a command answers to.
  const named = commands.filter((command) => command.label.toLowerCase().includes(text));
  const meant = commands.filter(
    (command) =>
      !named.includes(command) &&
      command.keywords?.some((word) => word.includes(text) || text.includes(word)),
  );
  return [...named, ...meant].slice(0, limit);
}

/** Splits a label around the first match, so the matched resource can be bold. */
export function splitMatch(label: string, search: string): [string, string, string] {
  const text = search.trim().toLowerCase();
  const start = text ? label.toLowerCase().indexOf(text) : -1;
  if (start === -1) return [label, '', ''];
  const end = start + text.length;
  return [label.slice(0, start), label.slice(start, end), label.slice(end)];
}
