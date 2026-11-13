/** Render. */
import type { ProviderEntry, ProviderSection } from './types';

/** Render’s tab: the sections each of its resources gets. */
const SECTIONS: ProviderSection[] = [
  {
    id: 'overview',
    label: 'Overview',
    about: 'This service on Render: its plan, region, build and start commands.',
    refs: ['srv-', 'crn-'],
    views: [
      { op: 'retrieveService', title: 'Service' },
      { op: 'listInstances', title: 'Instances', about: 'What’s running now' },
    ],
    actions: [
      { op: 'updateService', label: 'Change settings' },
      { op: 'restartService', label: 'Restart' },
      { op: 'suspendService', label: 'Suspend' },
      { op: 'resumeService', label: 'Resume' },
    ],
  },
  {
    id: 'deploys',
    label: 'Deploys',
    about: 'Every deploy of this service, newest first.',
    refs: ['srv-', 'crn-'],
    views: [{ op: 'listDeploys', title: 'Deploys', args: { limit: 20 } }],
    actions: [{ op: 'createDeploy', label: 'Deploy' }],
  },
  {
    id: 'cron',
    label: 'Cron job',
    about: 'When this job runs, and running it now.',
    refs: ['crn-'],
    views: [{ op: 'listJob', title: 'Runs' }],
    actions: [
      { op: 'runCronJob', label: 'Run now' },
      { op: 'cancelCronJobRun', label: 'Cancel the run' },
    ],
  },
  {
    id: 'events',
    label: 'Events',
    about: 'What happened to the service: deploys, restarts, scaling.',
    refs: ['srv-', 'crn-'],
    views: [{ op: 'listEvents', title: 'Events', args: { limit: 20 } }],
    actions: [],
  },
  {
    id: 'networking',
    label: 'Networking',
    about: 'Custom domains, redirects and headers, and the addresses it calls out from.',
    refs: ['srv-', 'crn-'],
    views: [
      { op: 'listCustomDomains', title: 'Custom domains' },
      { op: 'listRoutes', title: 'Redirects and rewrites' },
      { op: 'listHeaders', title: 'Headers' },
      { op: 'retrieveServiceOutboundIps', title: 'Outbound IPs' },
    ],
    actions: [],
  },
  {
    id: 'disks',
    label: 'Disks',
    about: 'The disk attached to this service.',
    refs: ['srv-', 'crn-'],
    views: [{ op: 'listDisks', title: 'Disks' }],
    actions: [],
  },
  {
    id: 'overview',
    label: 'Overview',
    about: 'This Postgres database on Render: its plan, version and status.',
    refs: ['dpg-'],
    views: [
      { op: 'retrievePostgres', title: 'Database', about: 'Connection details aren’t shown' },
    ],
    actions: [
      { op: 'updatePostgres', label: 'Change settings' },
      { op: 'restartPostgres', label: 'Restart' },
      { op: 'suspendPostgres', label: 'Suspend' },
      { op: 'resumePostgres', label: 'Resume' },
    ],
  },
  {
    id: 'performance',
    label: 'Performance',
    about: 'The slowest queries, table scans, running processes and table sizes.',
    refs: ['dpg-'],
    views: [
      { op: 'listPostgresTopQueries', title: 'Top queries' },
      { op: 'listPostgresTableScans', title: 'Table scans' },
      { op: 'listPostgresProcesses', title: 'Processes' },
      { op: 'listPostgresSizes', title: 'Sizes' },
    ],
    actions: [],
  },
  {
    id: 'recovery',
    label: 'Recovery',
    about: 'Restoring to a point in time, and switching to the standby.',
    refs: ['dpg-'],
    views: [{ op: 'retrievePostgresRecoveryInfo', title: 'Recovery window' }],
    actions: [
      { op: 'recoverPostgres', label: 'Restore to a point in time' },
      { op: 'failoverPostgres', label: 'Switch to the standby' },
    ],
  },
  {
    id: 'overview',
    label: 'Overview',
    about: 'This Key Value instance on Render: its plan and status.',
    refs: ['red-'],
    views: [
      { op: 'retrieveKeyValue', title: 'Key Value', about: 'Connection details aren’t shown' },
    ],
    actions: [
      { op: 'updateKeyValue', label: 'Change settings' },
      { op: 'suspendKeyValue', label: 'Suspend' },
      { op: 'resumeKeyValue', label: 'Resume' },
    ],
  },
];

export const render: ProviderEntry = {
  id: 'render',
  name: 'Render',
  facts: [
    {
      title: 'Service',
      op: 'retrieveService',
      on: ['web', 'service', 'job'],
      rows: [
        { label: 'Type', path: 'type' },
        { label: 'Region', path: 'serviceDetails.region', as: 'code' },
        { label: 'Plan', path: 'serviceDetails.plan' },
        { label: 'Runtime', path: 'serviceDetails.runtime' },
        { label: 'Repository', path: 'repo', as: 'code' },
        { label: 'Branch', path: 'branch', as: 'code' },
        { label: 'Auto-deploy', path: 'autoDeploy' },
        { label: 'Created', path: 'createdAt', as: 'ago' },
      ],
    },
    {
      title: 'Database',
      op: 'retrievePostgres',
      on: ['database'],
      rows: [
        { label: 'Status', path: 'status' },
        { label: 'Version', path: 'version' },
        { label: 'Plan', path: 'plan' },
        { label: 'Region', path: 'region', as: 'code' },
        { label: 'Disk', path: 'diskSizeGB', as: 'gb' },
        { label: 'Created', path: 'createdAt', as: 'ago' },
      ],
    },
    {
      title: 'Events',
      op: 'listEvents',
      on: ['web', 'service', 'job'],
      args: { limit: 8 },
      list: '',
      events: true,
      columns: [
        { label: 'Event', path: 'event.type' },
        { label: 'When', path: 'event.timestamp', as: 'ago' },
      ],
    },
  ],
  charts: {},
  logo: {
    colour: 'var(--text)',
    path: 'M18.263.007c-3.121-.147-5.744 2.109-6.192 5.082-.018.138-.045.272-.067.405-.696 3.703-3.936 6.507-7.827 6.507-1.388 0-2.691-.356-3.825-.979a.2024.2024 0 0 0-.302.178V24H12v-8.999c0-1.656 1.338-3 2.987-3h2.988c3.382 0 6.103-2.817 5.97-6.244-.12-3.084-2.61-5.603-5.682-5.75',
  },
  canImport: true,
  catalogue: { description: 'Web services and cron jobs', category: 'hosting' },
  connect: {
    intro: 'Web services, workers, cron jobs and Postgres. Hoista sees your Render services here.',
    token_steps: 'Account Settings → API Keys → Create API Key. Name it Hoista.',
    token_label: 'Render API key',
    token_placeholder: 'rnd_…',
    sign_in: null,
    permissions: [],
  },
  pages: {
    colour: '#111111',
    signupOptions: ['Sign up with GitHub', 'Sign up with email'],
    tokensNav: 'Account settings',
    tokensTitle: 'API Keys',
    createLabel: 'Create API Key',
    fields: [{ label: 'Name', value: 'hoista' }],
    tokenSteps: [
      'Open Account settings › API Keys',
      'Click Create API key. Naming it hoista is optional',
      'Copy the key. It’s shown once.',
    ],
    usedFor: 'Web services and cron jobs',
    plan: 'Free services sleep after 15 minutes idle',
  },
  words: {
    product: (kind) =>
      kind === 'web'
        ? 'Web service'
        : kind === 'job'
          ? 'Cron job'
          : kind === 'database'
            ? 'Database'
            : 'Service',
    compute: 'Compute',
    storage: 'Storage',
    vcpu: false,
  },
  // Each kind of resource has its own page, by the id's prefix.
  console: (reference) => {
    const page: Record<string, string> = { srv: 'web', crn: 'cron', dpg: 'd', red: 'r' };
    const kind = page[reference.split('-')[0] ?? ''];
    return kind
      ? `https://dashboard.render.com/${kind}/${encodeURIComponent(reference)}`
      : 'https://dashboard.render.com';
  },
  sampleToken: 'rnd_••••••••••d42b',
  projectsWord: 'Services',
  sections: SECTIONS,
  accountWord: 'Workspace',
  groupWord: 'project',
  billing: 'https://dashboard.render.com/billing',
  dashboard: () => 'https://dashboard.render.com',
  accountFacts: [
    {
      title: 'Workspace',
      op: 'retrieveOwner',
      rows: [
        { label: 'Name', path: 'name' },
        { label: 'Email', path: 'email' },
        { label: 'Kind', path: 'type' },
      ],
    },
  ],
  tile: { for: 'Web services, workers, cron jobs and Postgres', group: 'hosting' },
};
