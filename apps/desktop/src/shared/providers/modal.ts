/** Modal. */
import type { ProviderEntry, ProviderSection } from './types';

/**
 * Modal's tab, by what the resource is: an app (`app:`) or a volume
 * (`volume:`). Operations are Modal's own gRPC methods, from Hoista's
 * catalog of them.
 */
const SECTIONS: ProviderSection[] = [
  {
    id: 'overview',
    label: 'Overview',
    about: 'This app on Modal: when it was deployed, its functions and web endpoints.',
    refs: ['app:'],
    views: [
      { op: 'AppGetLifecycle', title: 'Lifecycle' },
      { op: 'AppGetLayout', title: 'Functions and endpoints', field: 'app_layout' },
      { op: 'AppGetTags', title: 'Tags', empty: 'No tags.' },
    ],
    actions: [{ op: 'AppSetTags', label: 'Set tags' }],
  },
  {
    id: 'deployments',
    label: 'Deployments',
    about: 'Every deployment of the app, newest first, and rolling back to one.',
    refs: ['app:'],
    views: [
      {
        op: 'AppDeploymentHistory',
        title: 'Deployments',
        field: 'app_deployment_histories',
        empty: 'Not deployed yet.',
        rowActions: [
          { op: 'AppRollback', label: 'Roll back to this', arg: 'version', from: 'version' },
        ],
      },
    ],
    actions: [],
  },
  {
    id: 'volume',
    label: 'Volume',
    about: 'This volume on Modal. Files on it are read and written by your apps.',
    refs: ['volume:'],
    views: [],
    actions: [{ op: 'VolumeRename', label: 'Rename' }],
  },
  {
    id: 'environment',
    label: 'Environment',
    about: 'The Modal environment it’s in: its limits and spend, and the secrets it holds.',
    views: [
      { op: 'EnvironmentList', title: 'Environments', field: 'items' },
      {
        op: 'SecretList',
        title: 'Secrets',
        field: 'items',
        about: 'Names only',
        empty: 'No secrets.',
      },
    ],
    actions: [],
  },
];

/**
 * Modal's dashboard for a resource. Its addresses use the workspace's name,
 * which Hoista doesn't keep, so this opens the apps or storage page and
 * Modal shows the signed-in workspace.
 */
function consoleOf(reference: string): string {
  return reference.startsWith('volume:') ? 'https://modal.com/storage' : 'https://modal.com/apps';
}

export const modal: ProviderEntry = {
  id: 'modal',
  name: 'Modal',
  facts: [],
  charts: {},
  logo: {
    colour: 'var(--text)',
    path: 'M4.89 5.57 0 14.002l2.521 4.4h5.05l4.396-7.718 4.512 7.709 4.996.037L24 14.057l-4.857-8.452-5.073-.015-2.076 3.598L9.94 5.57Zm.837.729h3.787l1.845 3.252H7.572Zm9.189.021 3.803.012 4.228 7.355-3.736-.027zm-9.82.346L6.94 9.914l-4.209 7.389-1.892-3.3Zm9.187.014 4.297 7.343-1.892 3.282-4.3-7.344zm-6.713 3.6h3.79l-4.212 7.394H3.361Zm11.64 4.109 3.74.027-1.893 3.281-3.74-.027z',
  },
  canImport: true,
  catalogue: { description: 'Serverless GPUs for Python apps', category: 'gpu' },
  connect: {
    intro:
      'Your Modal apps and volumes in every environment. Hoista shows each app, whether it’s running or scaled to zero, and what the workspace has cost.',
    token_steps:
      'Settings → API Tokens → New Token. Paste the line it shows (modal token set --token-id ak-… --token-secret as-…); Hoista takes both parts from it.',
    token_label: 'Modal token ID and secret',
    token_placeholder: 'ak-… as-…',
    sign_in: null,
    permissions: [],
  },
  pages: {
    colour: '#111111',
    signupOptions: ['Sign up with GitHub', 'Sign up with Google'],
    tokensNav: 'Settings',
    tokensTitle: 'API Tokens',
    createLabel: 'New Token',
    fields: [{ label: 'Name', value: 'hoista' }],
    tokenSteps: [
      'Open Settings › API Tokens',
      'Click New Token and name it hoista',
      'Copy the line it shows. It has the ID (ak-…) and the secret (as-…), shown once.',
    ],
    usedFor: 'GPU functions, web endpoints and batch jobs',
    plan: '$30 of compute free each month on the Starter plan',
    headsUp: 'Paste the whole line Modal shows: Hoista needs both the token ID and the secret.',
  },
  words: {
    term: (kind) => (kind === 'bucket' ? 'Volume' : 'App'),
    product: (kind) => (kind === 'bucket' ? 'Volume' : 'App'),
    compute: 'GPU',
    storage: 'Volume',
    vcpu: false,
  },
  console: consoleOf,
  sampleToken: 'ak-•••••• as-••••••d42b',
  projectsWord: 'Apps and volumes',
  sections: SECTIONS,
  access: { read: 'Token', write: 'Token', delete: 'Token' },
  accountWord: 'Workspace',
  billing: 'https://modal.com/settings/usage',
  dashboard: () => 'https://modal.com/apps',
  fieldWords: {
    labels: {
      app_id: 'App ID',
      n_running_tasks: 'Running containers',
      created_at: 'Created',
      deployed_at: 'Deployed',
      deployed_by: 'By',
      client_version: 'Modal version',
      environment_name: 'Environment',
      metered_cost: 'Metered cost (USD)',
      max_concurrent_gpus: 'GPU limit',
      current_concurrent_gpus: 'GPUs in use',
    },
    values: {
      APP_STATE_DEPLOYED: 'Deployed',
      APP_STATE_EPHEMERAL: 'Running from a terminal',
      APP_STATE_DETACHED: 'Running, detached',
      APP_STATE_STOPPED: 'Stopped',
      APP_STATE_INITIALIZING: 'Starting',
    },
    units: {},
  },
  tile: { for: 'Serverless GPU apps, functions and volumes', group: 'gpu' },
};
