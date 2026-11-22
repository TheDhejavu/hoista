/** Hetzner Cloud. */
import type { ProviderEntry, ProviderSection, ServerChoice } from './types';

/**
 * Hetzner's tab, by what the resource is: a server (`server:`) or a volume
 * (`volume:`). Operations come from the Hetzner Cloud API.
 */
const SECTIONS: ProviderSection[] = [
  {
    id: 'overview',
    label: 'Overview',
    about: 'This server on Hetzner: its type, image, addresses and protection.',
    refs: ['server:'],
    views: [{ op: 'getServer', title: 'Server', field: 'server' }],
    actions: [
      { op: 'softRebootServer', label: 'Reboot' },
      { op: 'createImageFromServer', label: 'Take a snapshot', args: { type: 'snapshot' } },
      { op: 'enableAndConfigureBackupsForServer', label: 'Turn on backups' },
      { op: 'changeTypeOfServer', label: 'Change type' },
    ],
  },
  {
    id: 'activity',
    label: 'Activity',
    about: 'What was done to the server lately, such as power changes and backups.',
    refs: ['server:'],
    views: [
      {
        op: 'listActionsForServer',
        title: 'Actions',
        field: 'actions',
        args: { sort: ['id:desc'], per_page: 20 },
        empty: 'Nothing done to this server yet.',
      },
    ],
    actions: [],
  },
  {
    id: 'overview',
    label: 'Overview',
    about: 'This volume: its size, location and the server it’s attached to.',
    refs: ['volume:'],
    views: [{ op: 'getVolume', title: 'Volume', field: 'volume' }],
    actions: [{ op: 'resizeVolume', label: 'Resize' }],
  },
];

const text = (item: ServerChoice, key: string): string => {
  const value = item[key];
  return typeof value === 'string' || typeof value === 'number' ? String(value) : '';
};

/** `cx23 · 2 vCPUs · 4 GB memory · 40 GB disk`, and its CPU kind. */
function typeLabel(item: ServerChoice): string {
  const cpu = text(item, 'cpu_type') === 'dedicated' ? 'dedicated' : 'shared';
  const arm = text(item, 'architecture') === 'arm' ? ', Arm' : '';
  return `${text(item, 'name')} · ${text(item, 'cores')} vCPUs (${cpu}${arm}) · ${text(item, 'memory')} GB memory · ${text(item, 'disk')} GB disk`;
}

function offeredAt(item: ServerChoice, location: string): boolean {
  if (item.deprecated === true || (item.deprecation !== null && item.deprecation !== undefined)) {
    return false;
  }
  const prices = Array.isArray(item.prices) ? (item.prices as ServerChoice[]) : [];
  return prices.some((price) => text(price, 'location') === location);
}

export const hetzner: ProviderEntry = {
  id: 'hetzner',
  name: 'Hetzner',
  facts: [
    {
      title: 'Server',
      op: 'getServer',
      on: ['server'],
      refs: ['server:'],
      rows: [
        { label: 'Status', path: 'server.status' },
        { label: 'Type', path: 'server.server_type.name', as: 'code' },
        { label: 'vCPUs', path: 'server.server_type.cores' },
        { label: 'Memory', path: 'server.server_type.memory', as: 'gb' },
        { label: 'Disk', path: 'server.server_type.disk', as: 'gb' },
        { label: 'Image', path: 'server.image.description' },
        { label: 'IPv4', path: 'server.public_net.ipv4.ip', as: 'code' },
        { label: 'IPv6', path: 'server.public_net.ipv6.ip', as: 'code' },
        { label: 'Location', path: 'server.datacenter.location.city' },
        { label: 'Delete protection', path: 'server.protection.delete', as: 'bool' },
        { label: 'Created', path: 'server.created', as: 'ago' },
      ],
    },
    {
      title: 'Volume',
      op: 'getVolume',
      on: ['bucket'],
      refs: ['volume:'],
      rows: [
        { label: 'Status', path: 'volume.status' },
        { label: 'Size', path: 'volume.size', as: 'gb' },
        { label: 'Location', path: 'volume.location.city' },
        { label: 'Device', path: 'volume.linux_device', as: 'code' },
        { label: 'File system', path: 'volume.format', as: 'code' },
        { label: 'Created', path: 'volume.created', as: 'ago' },
      ],
    },
  ],
  charts: { refs: ['server:'] },
  logo: {
    colour: '#D50C2D',
    fillRule: 'evenodd',
    path: 'M0 0h24v24H0zM5.5 5v14h3.25v-5.4h6.5V19h3.25V5h-3.25v5.4h-6.5V5z',
  },
  canImport: true,
  catalogue: { description: 'Linux servers you rent by the hour', category: 'hosting' },
  connect: {
    intro:
      'The servers and volumes in one Hetzner project. Hoista shows each, how busy it is, and can turn it off or on.',
    token_steps:
      'Open the project → Security → API tokens → Generate API token → Read & Write. Name it Hoista.',
    token_label: 'Hetzner API token',
    token_placeholder: '64 letters and digits',
    sign_in: null,
    permissions: [],
  },
  pages: {
    colour: '#D50C2D',
    signupOptions: ['Sign up with email'],
    tokensNav: 'Security',
    tokensTitle: 'API tokens',
    createLabel: 'Generate API token',
    fields: [
      { label: 'Description', value: 'hoista' },
      { label: 'Permissions', value: 'Read & Write' },
    ],
    tokenSteps: [
      'Open your project in the Hetzner Console, then Security › API tokens',
      'Click Generate API token, name it hoista and choose Read & Write',
      'Copy the token. It’s shown once.',
    ],
    usedFor: 'Linux servers and the volumes attached to them',
    plan: 'Billed by the hour, up to a monthly cap per server',
    headsUp:
      'A token reaches one project; connect each project on its own. A server that’s off is still billed until it’s deleted.',
  },
  words: {
    term: (kind) => (kind === 'bucket' ? 'Volume' : 'Server'),
    product: (kind) => (kind === 'bucket' ? 'Volume' : 'Server'),
    compute: 'CPU',
    storage: 'Volume',
    vcpu: false,
  },
  console: () => 'https://console.hetzner.com/projects',
  sampleToken: '••••••••••••••••a7Qx',
  projectsWord: 'Servers and volumes',
  sections: SECTIONS,
  access: { read: 'Read', write: 'Read & Write', delete: 'Read & Write' },
  accountWord: 'Project',
  billing: 'https://accounts.hetzner.com/invoice',
  dashboard: () => 'https://console.hetzner.com/projects',
  fieldWords: {
    labels: {
      server_type: 'Type',
      public_net: 'Public network',
      dns_ptr: 'Reverse DNS',
      linux_device: 'Device',
      rescue_enabled: 'Rescue system',
      backup_window: 'Backup window',
      outgoing_traffic: 'Outgoing traffic',
      ingoing_traffic: 'Incoming traffic',
      included_traffic: 'Included traffic',
    },
    values: {
      running: 'Running',
      off: 'Off',
      initializing: 'Setting up',
      starting: 'Starting',
      stopping: 'Stopping',
      rebuilding: 'Rebuilding',
      migrating: 'Moving',
      deleting: 'Deleting',
      available: 'Available',
      creating: 'Creating',
      success: 'Done',
      error: 'Failed',
    },
    units: { size: 'GB', disk: 'GB', memory: 'GB' },
  },
  tile: { for: 'Linux servers and volumes', group: 'hosting' },
  servers: {
    noun: 'Server',
    locations: {
      op: 'listLocations',
      list: 'locations',
      value: (item) => text(item, 'name'),
      label: (item) => `${text(item, 'city')}, ${text(item, 'country')} (${text(item, 'name')})`,
    },
    types: {
      op: 'listServerTypes',
      args: { per_page: 50 },
      list: 'server_types',
      value: (item) => text(item, 'name'),
      label: typeLabel,
      offered: offeredAt,
    },
  },
};
