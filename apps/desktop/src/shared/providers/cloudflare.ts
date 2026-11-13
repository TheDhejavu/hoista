/** Cloudflare. */
import type { ProviderEntry, ProviderSection } from './types';

/** The ten permissions Hoista's Cloudflare adapter needs. */
const PERMISSIONS = [
  { scope: 'Account', resource: 'Workers Scripts', access: 'Edit', optional_for: null },
  { scope: 'Account', resource: 'Workers R2 Storage', access: 'Edit', optional_for: null },
  { scope: 'Account', resource: 'Account Settings', access: 'Read', optional_for: null },
  { scope: 'Account', resource: 'Account Analytics', access: 'Read', optional_for: null },
  { scope: 'Account', resource: 'Workers Tail', access: 'Read', optional_for: null },
  { scope: 'Account', resource: 'Billing', access: 'Read', optional_for: null },
  { scope: 'Account', resource: 'D1', access: 'Edit', optional_for: 'D1 databases' },
  { scope: 'Zone', resource: 'Zone', access: 'Edit', optional_for: 'your own domain' },
  { scope: 'Zone', resource: 'DNS', access: 'Edit', optional_for: 'your own domain' },
  { scope: 'Zone', resource: 'Workers Routes', access: 'Edit', optional_for: 'your own domain' },
];

/**
 * Cloudflare's tab for each kind of resource: a Worker (and a Cron
 * Trigger Worker), a Pages project, a domain (zone) and an R2 bucket. Each
 * view is a read Cloudflare answers inside `result`.
 */
const SECTIONS: ProviderSection[] = [
  {
    id: 'overview',
    label: 'Overview',
    about: 'How this Worker runs on Cloudflare: its settings, bindings and address.',
    refs: ['worker:'],
    views: [
      {
        op: 'workerScriptGetSettings',
        title: 'Settings and bindings',
        about: 'Compatibility date, bindings, limits, logs',
        field: 'result',
      },
      {
        op: 'workerScriptGetSubdomain',
        title: 'workers.dev address',
        about: 'Whether it answers on workers.dev',
        field: 'result',
      },
    ],
    actions: [{ op: 'workerScriptSettingsPatchSettings', label: 'Change settings' }],
  },
  {
    id: 'deployments',
    label: 'Deployments and versions',
    about:
      'What’s live now, and every version uploaded. A deployment can split traffic between versions.',
    refs: ['worker:'],
    views: [
      {
        op: 'workerDeploymentsListDeployments',
        title: 'Deployments',
        about: 'Newest first',
        field: 'result',
      },
      { op: 'workerVersionsListVersions', title: 'Versions', field: 'result' },
    ],
    actions: [{ op: 'workerDeploymentsCreateDeployment', label: 'Roll back or split traffic' }],
  },
  {
    id: 'triggers',
    label: 'Triggers and domains',
    about: 'What runs this Worker: its schedules, and the custom domains in the account.',
    refs: ['worker:'],
    views: [
      {
        op: 'workerCronTriggerGetCronTriggers',
        title: 'Cron Triggers',
        about: 'Schedules, in UTC',
        field: 'result',
      },
      { op: 'workersDomainsList', title: 'Custom domains', field: 'result' },
    ],
    actions: [{ op: 'workerCronTriggerUpdateCronTriggers', label: 'Change schedules' }],
  },
  {
    id: 'storage',
    label: 'Storage',
    about: 'D1 databases, KV namespaces, R2 buckets and Queues in this account.',
    refs: ['worker:'],
    views: [
      { op: 'd1ListDatabases', title: 'D1 databases', field: 'result' },
      { op: 'workersKvNamespaceListNamespaces', title: 'KV namespaces', field: 'result' },
      { op: 'r2ListBuckets', title: 'R2 buckets', field: 'result' },
      { op: 'queuesList', title: 'Queues', field: 'result' },
    ],
    actions: [{ op: 'd1CreateDatabase', label: 'New D1 database' }],
  },
  {
    id: 'overview',
    label: 'Overview',
    about: 'This Pages project: where it builds from, its settings and its domains.',
    refs: ['pages:'],
    views: [
      { op: 'pagesProjectGetProject', title: 'Project', field: 'result' },
      {
        op: 'pagesDeploymentGetDeployments',
        title: 'Deployments',
        about: 'Newest first',
        field: 'result',
      },
      { op: 'pagesDomainsGetDomains', title: 'Custom domains', field: 'result' },
    ],
    actions: [
      { op: 'pagesProjectUpdateProject', label: 'Change settings' },
      { op: 'pagesPurgeBuildCache', label: 'Clear build cache' },
    ],
  },
  {
    id: 'overview',
    label: 'Overview',
    about: 'This domain on Cloudflare: its status, plan and name servers.',
    refs: ['zone:'],
    views: [
      { op: 'zones0Get', title: 'Domain', field: 'result' },
      { op: 'dnsRecordsForAZoneGetUsage', title: 'DNS records used', field: 'result' },
    ],
    actions: [
      { op: 'zones0Patch', label: 'Change settings' },
      { op: 'zonePurge', label: 'Purge cache' },
    ],
  },
  {
    id: 'dns',
    label: 'DNS',
    about: 'The domain’s DNS records.',
    refs: ['zone:'],
    views: [{ op: 'dnsRecordsForAZoneListDnsRecords', title: 'Records', field: 'result' }],
    actions: [
      { op: 'dnsRecordsForAZoneCreateDnsRecord', label: 'Add a record' },
      { op: 'dnsRecordsForAZoneBatchDnsRecords', label: 'Change several records' },
    ],
  },
  {
    id: 'certificates',
    label: 'Certificates and hostnames',
    about: 'Certificates Cloudflare issued for the domain, and custom hostnames on it.',
    refs: ['zone:'],
    views: [
      { op: 'certificatePacksListCertificatePacks', title: 'Certificates', field: 'result' },
      {
        op: 'customHostnameForAZoneListCustomHostnames',
        title: 'Custom hostnames',
        field: 'result',
      },
    ],
    actions: [],
  },
  {
    id: 'workers',
    label: 'Workers on this domain',
    about: 'Routes that send this domain’s traffic to a Worker.',
    refs: ['zone:'],
    views: [{ op: 'workerRoutesListRoutes', title: 'Routes', field: 'result' }],
    actions: [],
  },
  {
    id: 'email',
    label: 'Email Routing',
    about: 'Forwarding mail sent to this domain.',
    refs: ['zone:'],
    views: [
      { op: 'emailRoutingSettingsGetEmailRoutingSettings', title: 'Settings', field: 'result' },
      { op: 'emailRoutingRoutingRulesListRoutingRules', title: 'Rules', field: 'result' },
    ],
    actions: [{ op: 'emailRoutingSettingsEnableEmailRoutingDns', label: 'Turn on Email Routing' }],
  },
  {
    id: 'overview',
    label: 'Overview',
    about: 'This D1 database: its size, tables and where it runs.',
    refs: ['d1:'],
    views: [{ op: 'd1GetDatabase', title: 'Database', field: 'result' }],
    actions: [
      { op: 'd1QueryDatabase', label: 'Run SQL' },
      { op: 'd1ExportDatabase', label: 'Export' },
    ],
  },
  {
    id: 'backups',
    label: 'Time Travel',
    about: 'Restore the database to any minute in the last 30 days.',
    refs: ['d1:'],
    views: [{ op: 'd1TimeTravelGetBookmark', title: 'Current bookmark', field: 'result' }],
    actions: [{ op: 'd1TimeTravelRestore', label: 'Restore' }],
  },
  {
    id: 'overview',
    label: 'Overview',
    about: 'This KV namespace and the keys in it. Values are read one key at a time.',
    refs: ['kv:'],
    views: [
      { op: 'workersKvNamespaceGetANamespace', title: 'Namespace', field: 'result' },
      {
        op: 'workersKvNamespaceListANamespaceSKeys',
        title: 'Keys',
        field: 'result',
        empty: 'No keys yet.',
      },
    ],
    actions: [{ op: 'workersKvNamespaceRenameANamespace', label: 'Rename' }],
  },
  {
    id: 'overview',
    label: 'Overview',
    about: 'This R2 bucket: where it is and how it stores data.',
    refs: ['r2:'],
    views: [{ op: 'r2GetBucket', title: 'Bucket', field: 'result' }],
    actions: [{ op: 'r2PatchBucket', label: 'Change settings' }],
  },
  {
    id: 'access',
    label: 'Public access and domains',
    about: 'Whether the bucket is public on r2.dev, and the custom domains that serve it.',
    refs: ['r2:'],
    views: [
      { op: 'r2GetBucketPublicPolicy', title: 'r2.dev access', field: 'result' },
      { op: 'r2ListCustomDomains', title: 'Custom domains', field: 'result' },
    ],
    actions: [],
  },
  {
    id: 'rules',
    label: 'CORS and lifecycle',
    about: 'Which sites may read it, and when objects expire or move to cheaper storage.',
    refs: ['r2:'],
    views: [
      { op: 'r2GetBucketCorsPolicy', title: 'CORS', field: 'result' },
      { op: 'r2GetBucketLifecycleConfiguration', title: 'Lifecycle rules', field: 'result' },
      { op: 'r2GetBucketLockConfiguration', title: 'Object lock', field: 'result' },
    ],
    actions: [
      { op: 'r2PutBucketCorsPolicy', label: 'Change CORS' },
      { op: 'r2PutBucketLifecycleConfiguration', label: 'Change lifecycle rules' },
    ],
  },
];

export const cloudflare: ProviderEntry = {
  id: 'cloudflare',
  name: 'Cloudflare',
  facts: [
    {
      title: 'Worker',
      op: 'workerScriptGetSettings',
      refs: ['worker:'],
      rows: [
        { label: 'Compatibility date', path: 'compatibility_date', as: 'code' },
        { label: 'Compatibility flags', path: 'compatibility_flags', as: 'join' },
        { label: 'Logs', path: 'observability.enabled', as: 'bool' },
        { label: 'Usage model', path: 'usage_model' },
      ],
    },
    {
      title: 'Deployments',
      op: 'workerDeploymentsListDeployments',
      refs: ['worker:'],
      list: 'deployments',
      limit: 8,
      events: true,
      columns: [
        { label: 'Deployment', path: 'id', as: 'code' },
        { label: 'Source', path: 'source' },
        { label: 'When', path: 'created_on', as: 'ago' },
      ],
    },
    {
      title: 'Bucket',
      op: 'r2GetBucket',
      refs: ['r2:'],
      rows: [
        { label: 'Location', path: 'location', as: 'code' },
        { label: 'Storage class', path: 'storage_class' },
        { label: 'Created', path: 'creation_date', as: 'ago' },
      ],
    },
    {
      title: 'Domain',
      op: 'zones0Get',
      refs: ['zone:'],
      rows: [
        { label: 'Status', path: 'status' },
        { label: 'Plan', path: 'plan.name' },
        { label: 'Name servers', path: 'name_servers', as: 'join' },
        { label: 'Created', path: 'created_on', as: 'ago' },
      ],
    },
  ],
  charts: { refs: ['worker:', 'zone:'] },
  logo: {
    colour: '#F38020',
    path: 'M16.5088 16.8447c.1475-.5068.0908-.9707-.1553-1.3154-.2246-.3164-.6045-.499-1.0615-.5205l-8.6592-.1123a.1559.1559 0 0 1-.1333-.0713c-.0283-.042-.0351-.0986-.021-.1553.0278-.084.1123-.1484.2036-.1562l8.7359-.1123c1.0351-.0489 2.1601-.8868 2.5537-1.9136l.499-1.3013c.0215-.0561.0293-.1128.0147-.168-.5625-2.5463-2.835-4.4453-5.5499-4.4453-2.5039 0-4.6284 1.6177-5.3876 3.8614-.4927-.3658-1.1187-.5625-1.794-.499-1.2026.119-2.1665 1.083-2.2861 2.2856-.0283.31-.0069.6128.0635.894C1.5683 13.171 0 14.7754 0 16.752c0 .1748.0142.3515.0352.5273.0141.083.0844.1475.1689.1475h15.9814c.0909 0 .1758-.0645.2032-.1553l.12-.4268zm2.7568-5.5634c-.0771 0-.1611 0-.2383.0112-.0566 0-.1054.0415-.127.0976l-.3378 1.1744c-.1475.5068-.0918.9707.1543 1.3164.2256.3164.6055.498 1.0625.5195l1.8437.1133c.0557 0 .1055.0263.1329.0703.0283.043.0351.1074.0214.1562-.0283.084-.1132.1485-.204.1553l-1.921.1123c-1.041.0488-2.1582.8867-2.5527 1.914l-.1406.3585c-.0283.0713.0215.1416.0986.1416h6.5977c.0771 0 .1474-.0489.169-.126.1122-.4082.1757-.837.1757-1.2803 0-2.6025-2.125-4.727-4.7344-4.727',
  },
  canImport: false,
  catalogue: { description: 'Sites, APIs, file storage and domains', category: 'hosting' },
  connect: {
    intro: 'Hosts your sites and APIs, file storage and domains. The free plan is enough to start.',
    token_steps: 'My Profile → API Tokens → Create Token → Create Custom Token. Name it Hoista.',
    token_label: 'Cloudflare API token',
    token_placeholder: 'Paste your Cloudflare API token',
    sign_in: {
      on_their_page: 'Cloudflare lists the permissions Hoista asks for before you allow them.',
    },
    permissions: PERMISSIONS,
  },
  accountId: {
    label: 'Account ID',
    hint: 'The long ID in your dashboard address: dash.cloudflare.com/<account ID>.',
  },
  access: { read: 'Read', write: 'Edit', delete: 'Edit' },
  accountWord: 'Account',
  billing: 'https://dash.cloudflare.com/?to=/:account/billing',
  dashboard: () => 'https://dash.cloudflare.com',
  tile: { for: 'Sites, APIs and domains', group: 'hosting' },
  pages: {
    colour: '#F38020',
    signupOptions: ['Sign up with email', 'Continue with Google'],
    tokensNav: 'API Tokens',
    tokensTitle: 'User API Tokens',
    createLabel: 'Create Token',
    fields: [
      { label: 'Template', value: 'Edit Cloudflare Workers' },
      { label: 'Account', value: 'Your account' },
      { label: 'Zone', value: 'All zones' },
    ],
    tokenSteps: [
      'Open My Profile › API Tokens and click Create Token',
      'Use the Edit Cloudflare Workers template',
      'Continue, create it, then copy the token. It’s shown once.',
    ],
    usedFor: 'Sites, APIs, files and domains',
    plan: 'Free tier covers most small apps',
    headsUp:
      'Signed up with Google or GitHub? Cloudflare asks you to set a password before you can create a token. For apps a team shares, an Account API token keeps working if you leave.',
  },
  words: {
    term: (kind, ref) =>
      ref.startsWith('worker:')
        ? kind === 'job'
          ? 'Cron Worker'
          : 'Worker'
        : ref.startsWith('pages:')
          ? 'Pages project'
          : ref.startsWith('d1:')
            ? 'D1 database'
            : ref.startsWith('kv:')
              ? 'KV namespace'
              : ref.startsWith('r2:')
                ? 'R2 bucket'
                : ref.startsWith('zone:')
                  ? 'Zone'
                  : undefined,
    product: (kind, reference) =>
      reference?.startsWith('d1:')
        ? 'D1'
        : reference?.startsWith('kv:')
          ? 'Workers KV'
          : kind === 'bucket'
            ? 'R2'
            : kind === 'job'
              ? 'Cron Trigger'
              : kind === 'domain'
                ? 'Domain'
                : 'Workers',
    compute: 'Compute',
    storage: 'Storage',
    vcpu: false,
  },
  // Workers, buckets and databases have pages of their own; the rest opens the account.
  console: (reference, account) => {
    const base = `https://dash.cloudflare.com/${account}`;
    const [kind, ...rest] = reference.split(':');
    const name = encodeURIComponent(rest.at(-1) ?? '');
    switch (kind) {
      case 'worker':
        return `${base}/workers/services/view/${name}/production`;
      case 'pages':
        return `${base}/pages/view/${name}`;
      case 'r2':
        return `${base}/r2/${encodeURIComponent(rest[0] ?? 'default')}/buckets/${name}`;
      case 'd1':
        return `${base}/workers/d1/databases/${name}`;
      case 'kv':
        return `${base}/workers/kv/namespaces/${name}`;
      default:
        return base;
    }
  },
  sampleToken: 'cf_••••••••••••3a91',
  metricsRefs: ['worker:', 'zone:'],
  projectsWord: 'Workers and sites',
  sections: SECTIONS,
};
