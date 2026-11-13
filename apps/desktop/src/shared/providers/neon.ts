/** Neon. */
import type { ProviderEntry, ProviderSection } from './types';

/** Neon’s tab: the sections each of its resources gets. */
const SECTIONS: ProviderSection[] = [
  {
    id: 'overview',
    label: 'Overview',
    about: 'This Neon project: its region, Postgres version and settings.',
    views: [
      { op: 'getProject', title: 'Project', field: 'project' },
      {
        op: 'getProjectAdvisorSecurityIssues',
        title: 'Security advice',
        about: 'What Neon’s advisor flags',
      },
    ],
    actions: [],
  },
  {
    id: 'branches',
    label: 'Branches',
    about: 'Copies of the database for trying changes, made in seconds.',
    views: [{ op: 'listProjectBranches', title: 'Branches', field: 'branches' }],
    actions: [{ op: 'createProjectBranch', label: 'New branch' }],
  },
  {
    id: 'compute',
    label: 'Compute',
    about: 'The compute endpoints that serve each branch, and when they sleep.',
    views: [{ op: 'listProjectEndpoints', title: 'Computes', field: 'endpoints' }],
    actions: [],
  },
  {
    id: 'snapshots',
    label: 'Snapshots',
    about: 'Saved points to restore a branch from.',
    views: [{ op: 'listSnapshots', title: 'Snapshots', field: 'snapshots' }],
    actions: [],
  },
  {
    id: 'activity',
    label: 'Neon’s activity',
    about: 'What Neon did on this project, newest first.',
    views: [{ op: 'listProjectOperations', title: 'Operations', field: 'operations' }],
    actions: [],
  },
];

export const neon: ProviderEntry = {
  id: 'neon',
  name: 'Neon',
  facts: [
    {
      title: 'Project',
      op: 'getProject',
      rows: [
        { label: 'Postgres', path: 'project.pg_version' },
        { label: 'Region', path: 'project.region_id', as: 'code' },
        { label: 'Storage', path: 'project.synthetic_storage_size', as: 'bytes' },
        { label: 'Compute this month', path: 'project.compute_time_seconds', as: 'seconds' },
        { label: 'Written this month', path: 'project.written_data_bytes', as: 'bytes' },
        { label: 'Sent out this month', path: 'project.data_transfer_bytes', as: 'bytes' },
        { label: 'Created', path: 'project.created_at', as: 'ago' },
      ],
    },
    {
      title: 'Branches',
      op: 'listProjectBranches',
      list: 'branches',
      columns: [
        { label: 'Branch', path: 'name' },
        { label: 'State', path: 'current_state' },
        { label: 'Size', path: 'logical_size', as: 'bytes' },
        { label: 'Default', path: 'default', as: 'bool' },
      ],
    },
    {
      title: 'Computes',
      op: 'listProjectEndpoints',
      list: 'endpoints',
      columns: [
        { label: 'Host', path: 'host', as: 'code' },
        { label: 'State', path: 'current_state' },
        { label: 'Up to', path: 'autoscaling_limit_max_cu', as: 'cu' },
      ],
    },
    {
      title: 'Operations',
      op: 'listProjectOperations',
      args: { limit: 8 },
      list: 'operations',
      events: true,
      columns: [
        { label: 'Action', path: 'action' },
        { label: 'Status', path: 'status' },
        { label: 'When', path: 'created_at', as: 'ago' },
      ],
    },
  ],
  charts: { plan: 'https://neon.com/pricing' },
  logo: {
    colour: '#34D59A',
    path: 'M24 0V24l-9.365-8.045V24H0V0ZM2.942 21.087h8.751V9.563l9.365 8.204V2.919L2.942 2.914Z',
  },
  canImport: false,
  catalogue: { description: 'Postgres databases that scale to zero', category: 'databases' },
  connect: {
    intro: 'Postgres databases that sleep when idle, so they cost nothing while nobody visits.',
    token_steps: 'Account settings → API keys → Create new API key. Name it Hoista.',
    token_label: 'Neon API key',
    token_placeholder: 'napi_…',
    sign_in: null,
    permissions: [],
  },
  accountId: {
    label: 'Organization ID',
    hint: 'In Neon: Organization settings. It starts with org-.',
  },
  accountWord: 'Organization',
  groupWord: 'project',
  billing: 'https://console.neon.tech/app/billing',
  dashboard: () => 'https://console.neon.tech/app/projects',
  accountFacts: [
    {
      title: 'Organization',
      op: 'getOrganization',
      rows: [
        { label: 'Name', path: 'name' },
        { label: 'Handle', path: 'handle', as: 'code' },
        { label: 'Plan', path: 'plan' },
        { label: 'Created', path: 'created_at', as: 'ago' },
      ],
    },
  ],
  tile: { for: 'Postgres that scales to zero', group: 'databases' },
  pages: {
    colour: '#00C98B',
    signupOptions: ['Continue with GitHub', 'Continue with email'],
    tokensNav: 'API keys',
    tokensTitle: 'API keys',
    createLabel: 'Create new API key',
    fields: [{ label: 'Key name', value: 'hoista' }],
    tokenSteps: [
      'Open Settings › API keys',
      'Click Create new API key and name it hoista',
      'Copy the key. It’s shown once.',
    ],
    usedFor: 'Postgres that scales to zero',
    plan: 'Free plan included',
    headsUp: 'Exact usage figures need a paid Neon plan.',
  },
  words: {
    term: (kind) => (kind === 'database' ? 'Postgres database' : undefined),
    product: (kind) => (kind === 'database' ? 'Postgres' : ''),
    compute: 'Compute',
    storage: 'Storage',
    vcpu: false,
  },
  console: (reference) => {
    const project = reference.replace(/^project\//, '').split('/')[0];
    return project
      ? `https://console.neon.tech/app/projects/${encodeURIComponent(project)}`
      : 'https://console.neon.tech/app/projects';
  },
  sampleToken: 'napi_••••••••••d42b',
  sections: SECTIONS,
};
