/** Railway. */
import type { ProviderEntry, ProviderSection } from './types';

/**
 * Railway's tab: the things that exist at Railway for an app, each with
 * what Railway reports about it and what you can change. Operations come
 * from Railway's catalog; one it doesn't have is left out, and a section
 * with nothing left is too.
 */
const SECTIONS: ProviderSection[] = [
  {
    id: 'overview',
    label: 'Overview',
    about: 'How this service runs at Railway: its settings, limits and what deploys it.',
    views: [
      {
        op: 'serviceInstance',
        title: 'How it runs',
        about: 'Start command, region, replicas, schedule',
      },
      { op: 'serviceInstanceLimits', title: 'Limits', about: 'Most CPU and memory it may use' },
      { op: 'deploymentTriggers', title: 'Deploys from', about: 'Branches that deploy on push' },
      { op: 'platformStatus', title: 'Railway status', about: 'Whether Railway is having trouble' },
    ],
    actions: [
      { op: 'serviceInstanceRedeploy', label: 'Redeploy' },
      { op: 'serviceInstanceUpdate', label: 'Change settings', current: 'serviceInstance' },
      {
        op: 'serviceInstanceLimitsUpdate',
        label: 'Change limits',
        current: 'serviceInstanceLimits',
      },
      { op: 'serviceInstanceAutoDeployUpdate', label: 'Auto-deploy' },
    ],
  },
  {
    id: 'environments',
    label: 'Environments',
    about: 'Production, staging and the preview environments Railway makes for pull requests.',
    views: [
      {
        op: 'environments',
        title: 'Environments',
        rowActions: [
          { op: 'environmentRename', label: 'Rename' },
          { op: 'environmentDelete', label: 'Delete' },
        ],
      },
      {
        op: 'environmentStagedChanges',
        title: 'Changes waiting',
        about: 'Staged, not yet applied',
        empty: 'No changes waiting.',
      },
      {
        op: 'environmentPatches',
        title: 'Change history',
        about: 'What changed, when and by whom',
        empty: 'No changes yet.',
      },
    ],
    actions: [
      { op: 'environmentCreate', label: 'New environment' },
      { op: 'environmentPatchCommitStaged', label: 'Apply waiting changes' },
    ],
  },
  {
    id: 'storage',
    label: 'Volumes and backups',
    about: 'Disks attached to this service, and the backups Railway keeps of them.',
    special: 'volumes',
    views: [],
    actions: [
      { op: 'volumeCreate', label: 'Add a volume' },
      { op: 'bucketCreate', label: 'New bucket' },
    ],
  },
  {
    id: 'networking',
    label: 'Networking',
    about: 'TCP endpoints, the private network between services, and fixed outbound IPs.',
    views: [
      { op: 'tcpProxies', title: 'TCP endpoints', empty: 'No TCP endpoints.' },
      { op: 'privateNetworks', title: 'Private network', empty: 'Not on a private network.' },
      { op: 'egressGateways', title: 'Static outbound IPs', empty: 'No static outbound IPs.' },
    ],
    actions: [
      { op: 'egressGatewayAssociationCreate', label: 'Add static IPs' },
      { op: 'privateNetworkEndpointRename', label: 'Rename private address' },
      { op: 'egressGatewayAssociationsClear', label: 'Remove static IPs' },
    ],
    link: { tab: 'domains', label: 'Public addresses are on the Domains tab' },
  },
  {
    id: 'alerts',
    label: 'Alerts from Railway',
    about: 'What Railway has told you about, and the rules that decide it.',
    views: [
      { op: 'notificationDeliveries', title: 'Recent alerts', empty: 'No alerts lately.' },
      {
        op: 'notificationRules',
        title: 'Alert rules',
        empty: 'No alert rules.',
        rowActions: [{ op: 'notificationRuleDelete', label: 'Delete' }],
      },
    ],
    actions: [{ op: 'notificationRuleCreate', label: 'New alert rule' }],
  },
  {
    id: 'activity',
    label: 'Railway’s activity',
    about: 'Railway’s own record of who deployed, changed and restarted things.',
    views: [
      { op: 'events', title: 'Recent activity', empty: 'Nothing lately.' },
      { op: 'tracingStatus', title: 'Tracing', empty: 'Tracing is off.' },
    ],
    actions: [],
    link: { tab: 'activity', label: 'Hoista’s record of the same is on the Activity tab' },
  },
  {
    id: 'edge',
    label: 'Edge: CDN and firewall',
    about: 'Caching and protection in front of this service.',
    views: [],
    actions: [
      { op: 'enableServiceCdn', label: 'Turn on CDN' },
      { op: 'disableServiceCdn', label: 'Turn off CDN' },
      { op: 'purgeServiceCache', label: 'Purge cache' },
      { op: 'setServiceUnderAttackMode', label: 'Under-attack mode' },
      { op: 'updateServiceEdgeRules', label: 'Firewall rules' },
    ],
  },
  {
    id: 'flags',
    label: 'Feature flags',
    about: 'Railway Signals: flags your code reads, changed without a deploy.',
    views: [
      {
        op: 'signals',
        title: 'Flags',
        empty: 'No flags yet.',
        rowActions: [
          { op: 'signalDefaultSet', label: 'Set default', arg: 'input.name', from: 'name' },
          { op: 'signalDelete', label: 'Delete', arg: 'input.name', from: 'name' },
        ],
      },
    ],
    actions: [{ op: 'signalCreate', label: 'New flag' }],
  },
  {
    id: 'sandboxes',
    label: 'Sandboxes',
    about: 'Throwaway machines at Railway for trying things.',
    views: [
      {
        op: 'sandboxes',
        title: 'Sandboxes',
        empty: 'No sandboxes.',
        rowActions: [{ op: 'sandboxDestroy', label: 'Destroy' }],
      },
    ],
    actions: [{ op: 'sandboxCreate', label: 'New sandbox' }],
  },
  {
    id: 'templates',
    label: 'Templates and add-ons',
    about: 'Ready-made services, like databases and queues, you can add to this project.',
    views: [
      {
        op: 'workspaceTemplates',
        title: 'Your templates',
        empty: 'You haven’t made any templates.',
      },
    ],
    actions: [{ op: 'templateDeployV2', label: 'Deploy a template' }],
  },
];

export const railway: ProviderEntry = {
  id: 'railway',
  name: 'Railway',
  logo: {
    colour: 'var(--text)',
    path: 'M.113 10.27A13.026 13.026 0 000 11.48h18.23c-.064-.125-.15-.237-.235-.347-3.117-4.027-4.793-3.677-7.19-3.78-.8-.034-1.34-.048-4.524-.048-1.704 0-3.555.005-5.358.01-.234.63-.459 1.24-.567 1.737h9.342v1.216H.113v.002zm18.26 2.426H.009c.02.326.05.645.094.961h16.955c.754 0 1.179-.429 1.315-.96zm-17.318 4.28s2.81 6.902 10.93 7.024c4.855 0 9.027-2.883 10.92-7.024H1.056zM11.988 0C7.5 0 3.593 2.466 1.531 6.108l4.75-.005v-.002c3.71 0 3.849.016 4.573.047l.448.016c1.563.052 3.485.22 4.996 1.364.82.621 2.007 1.99 2.712 2.965.654.902.842 1.94.396 2.934-.408.914-1.289 1.458-2.353 1.458H.391s.099.42.249.886h22.748A12.026 12.026 0 0024 12.005C24 5.377 18.621 0 11.988 0z',
  },
  canImport: true,
  catalogue: { description: 'Servers, workers and databases', category: 'hosting' },
  connect: {
    intro: 'Hoista sees your Railway projects so you can watch or manage them here.',
    token_steps:
      'Account Settings → Tokens → Create token and pick your workspace. Hoista may ask for its ID: in Railway press ⌘K → Copy active workspace ID.',
    token_label: 'Railway token',
    token_placeholder: 'Paste your Railway token',
    sign_in: { on_their_page: 'You choose the workspace Hoista can see.' },
    permissions: [],
  },
  accountId: {
    label: 'Workspace ID',
    hint: 'In Railway, press ⌘K and choose “Copy active workspace ID”.',
  },
  accountWord: 'Workspace',
  groupWord: 'project',
  billing: 'https://railway.com/account/billing',
  dashboard: () => 'https://railway.com/dashboard',
  tile: { for: 'Services, workers and databases', group: 'hosting' },
  pages: {
    colour: '#853BCE',
    signupOptions: ['Continue with GitHub', 'Continue with email'],
    tokensNav: 'Tokens',
    tokensTitle: 'New Token',
    createLabel: 'Create',
    fields: [
      { label: 'Name', value: 'hoista' },
      { label: 'Workspace', value: 'Your workspace' },
    ],
    tokenSteps: [
      'Open Account › Tokens',
      'Name it hoista and pick your workspace (No workspace means every one)',
      'Click Create, then copy the token',
    ],
    usedFor: 'Services, workers and databases',
    plan: 'Trial credit, then a free plan with $1 a month',
  },
  words: {
    product: (kind) => (kind === 'database' ? 'Database' : kind === 'job' ? 'Cron' : 'Service'),
    // Railway measures CPU in vCPUs, so its compute is vCPU time.
    compute: 'CPU',
    storage: 'Disk',
    vcpu: true,
    redeploy: 'Runs the same commit again',
  },
  console: (reference) => {
    const [project, service] = reference.split('/');
    return service
      ? `https://railway.com/project/${project ?? ''}/service/${service}`
      : 'https://railway.com/dashboard';
  },
  project: (reference) => {
    const [project, service] = reference.split('/');
    return project && service ? `railway:${project}` : null;
  },
  sections: SECTIONS,
  tcpEndpoints: true,
  connectRepo: true,
  domainActions: { add: true, port: true, rename: true, certificate: true, remove: true },
  variables: { shared: true, redeploys: true },
  fieldWords: {
    labels: {
      numReplicas: 'Replicas',
      restartPolicyType: 'Restart',
      restartPolicyMaxRetries: 'Restart attempts',
      cpu: 'CPU',
      vCPUs: 'CPU',
      memoryGB: 'Memory',
      checkSuites: 'Waits for GitHub checks',
      healthcheckPath: 'Health check path',
      healthcheckTimeout: 'Health check timeout',
      startCommand: 'Start command',
      cronSchedule: 'Schedule',
      sleepApplication: 'Sleep when idle',
      isStable: 'Working normally',
      rootDirectory: 'Root folder',
      isEphemeral: 'Preview (pull request)',
      unmergedChangesCount: 'Changes waiting',
      eventTypes: 'Events',
      channelConfigs: 'Sends to',
      createdAt: 'Created',
      updatedAt: 'Updated',
    },
    values: {
      ON_FAILURE: 'On failure',
      ALWAYS: 'Always',
      NEVER: 'Never',
      SUCCESS: 'Active',
      REMOVED: 'Replaced',
      CRASHED: 'Crashed',
      FAILED: 'Failed',
      SLEEPING: 'Sleeping',
    },
    units: { cpu: 'vCPU', vCPUs: 'vCPU', memoryGB: 'GB', healthcheckTimeout: 's' },
  },
  databaseLogs: true,
  requests: { logs: 'httpLogs', byStatus: 'httpMetricsGroupedByStatus' },
  sampleToken: 'rw_•••••••••••91f3',
};
