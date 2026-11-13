/** Supabase. */
import type { ProviderEntry, ProviderSection } from './types';

/** Lists the project's own tables, biggest first, as Supabase's read-only user. */
const TABLES = `select schemaname as schema, relname as table, n_live_tup as rows,
  pg_size_pretty(pg_total_relation_size(relid)) as size
from pg_stat_user_tables
order by pg_total_relation_size(relid) desc
limit 100`;

/**
 * Supabase's tab: everything in the project a resource belongs to, each
 * with what Supabase reports and what you can change. Operations come from
 * Supabase's Management API; a section with nothing left is left out.
 */
const SECTIONS: ProviderSection[] = [
  {
    id: 'overview',
    label: 'Overview',
    about: 'The project this belongs to: its status, Postgres version, region and size.',
    views: [
      { op: 'getProject', title: 'Project', about: 'Status, region, Postgres version' },
      {
        op: 'getServicesHealth',
        title: 'Health',
        about: 'Each part of the project, as Supabase checks it',
        args: { services: 'db,auth,rest,realtime,storage,pooler' },
      },
      { op: 'getDiskUtilization', title: 'Disk', about: 'How much of the disk is used' },
    ],
    actions: [
      { op: 'restartProject', label: 'Restart' },
      { op: 'pauseProject', label: 'Pause' },
      { op: 'restoreProject', label: 'Resume' },
      { op: 'updateProject', label: 'Rename' },
    ],
  },
  {
    id: 'database',
    label: 'Database',
    about: 'Your tables, the changes applied to them, and how Postgres is set up.',
    views: [
      {
        op: 'readOnlyQuery',
        title: 'Tables',
        about: 'Rows and size of each, read as a read-only user',
        args: { query: TABLES },
      },
      {
        op: 'listMigrationHistory',
        title: 'Migrations',
        about: 'Applied with the Supabase CLI, newest last. Empty until the first one',
      },
      { op: 'getPoolerConfig', title: 'Connection pooling', about: 'How apps share connections' },
      { op: 'getPostgresConfig', title: 'Postgres settings' },
      { op: 'getReadonlyModeStatus', title: 'Read-only mode', about: 'On when the disk is full' },
    ],
    actions: [
      { op: 'readOnlyQuery', label: 'Run a read-only query' },
      { op: 'runQuery', label: 'Run SQL' },
      { op: 'applyMigration', label: 'Apply a migration' },
      { op: 'updatePoolerConfig', label: 'Change pooling' },
      { op: 'updatePostgresConfig', label: 'Change Postgres settings' },
      { op: 'generateTypescriptTypes', label: 'TypeScript types' },
    ],
  },
  {
    id: 'backups',
    label: 'Backups',
    about: 'The backups Supabase keeps of the database, and restoring one.',
    views: [
      { op: 'listAllBackups', title: 'Backups', field: 'backups' },
      { op: 'getBackupSchedule', title: 'Schedule', about: 'When the daily backup runs' },
    ],
    actions: [
      { op: 'restorePhysicalBackup', label: 'Restore a backup' },
      { op: 'restorePitrBackup', label: 'Restore to a moment' },
      { op: 'updateBackupSchedule', label: 'Change the schedule' },
    ],
  },
  {
    id: 'functions',
    label: 'Edge functions',
    about: 'Code Supabase runs close to your users, each at its own address.',
    views: [{ op: 'listAllFunctions', title: 'Functions' }],
    actions: [
      { op: 'updateFunction', label: 'Change a function' },
      { op: 'deleteFunction', label: 'Delete a function' },
    ],
  },
  {
    id: 'storage',
    label: 'Storage',
    about: 'Buckets of files your app uploads and serves.',
    views: [
      { op: 'listAllBuckets', title: 'Buckets' },
      { op: 'getStorageConfig', title: 'Storage settings', about: 'Largest file, features' },
    ],
    actions: [{ op: 'updateStorageConfig', label: 'Change storage settings' }],
  },
  {
    id: 'auth',
    label: 'Auth',
    about: 'How people sign in to your app: providers, emails, sessions and limits.',
    views: [
      { op: 'getAuthServiceConfig', title: 'Auth settings' },
      { op: 'listAllSsoProvider', title: 'SSO providers', field: 'items' },
      { op: 'listProjectTpaIntegrations', title: 'Other sign-in services' },
    ],
    actions: [
      { op: 'updateAuthServiceConfig', label: 'Change auth settings' },
      { op: 'createSsoProvider', label: 'Add an SSO provider' },
    ],
  },
  {
    id: 'api',
    label: 'Data API and Realtime',
    about: 'The API Supabase makes from your tables, and live updates.',
    views: [
      { op: 'getPostgrestServiceConfig', title: 'Data API', about: 'Schemas it serves, row limit' },
      { op: 'getRealtimeConfig', title: 'Realtime' },
    ],
    actions: [
      { op: 'updatePostgrestServiceConfig', label: 'Change the Data API' },
      { op: 'updateRealtimeConfig', label: 'Change Realtime' },
    ],
  },
  {
    id: 'network',
    label: 'Networking',
    about: 'Who may connect to the database, and whether connections must use SSL.',
    views: [
      { op: 'getNetworkRestrictions', title: 'Allowed addresses' },
      { op: 'getSslEnforcementConfig', title: 'SSL' },
      {
        op: 'listAllNetworkBans',
        title: 'Blocked addresses',
        about: 'After too many failed sign-ins',
      },
    ],
    actions: [
      { op: 'patchNetworkRestrictions', label: 'Change allowed addresses' },
      { op: 'updateSslEnforcementConfig', label: 'Require SSL' },
      { op: 'deleteNetworkBans', label: 'Unblock addresses' },
    ],
  },
  {
    id: 'domains',
    label: 'Domains',
    about: 'Your own address for the project, instead of the supabase.co one.',
    views: [
      { op: 'getHostnameConfig', title: 'Custom domain' },
      { op: 'getVanitySubdomainConfig', title: 'Vanity subdomain' },
    ],
    actions: [
      { op: 'updateHostnameConfig', label: 'Add a custom domain' },
      { op: 'verifyDnsConfig', label: 'Check DNS' },
      { op: 'activateCustomHostname', label: 'Turn on the domain' },
    ],
  },
  {
    id: 'secrets',
    label: 'Secrets and keys',
    about:
      'Names of the secrets your edge functions read, and the project’s keys. Values are never shown.',
    views: [
      { op: 'listAllSecrets', title: 'Secrets', about: 'Names only' },
      { op: 'getProjectSigningKeys', title: 'Signing keys', field: 'keys' },
      {
        op: 'getProjectLegacyApiKeys',
        title: 'Legacy keys',
        about: 'Whether anon and service_role work',
      },
    ],
    actions: [{ op: 'bulkDeleteSecrets', label: 'Delete secrets' }],
  },
  {
    id: 'branches',
    label: 'Branches',
    about: 'Copies of the database for trying changes before they reach production.',
    views: [
      { op: 'listAllBranches', title: 'Branches' },
      { op: 'listActionRuns', title: 'Branch runs' },
    ],
    actions: [
      { op: 'createBranch', label: 'New branch' },
      { op: 'mergeBranch', label: 'Merge a branch' },
      { op: 'deleteBranch', label: 'Delete a branch' },
    ],
  },
  {
    id: 'advisors',
    label: 'Advisors',
    about: 'What Supabase thinks could be safer or faster.',
    views: [
      { op: 'getSecurityAdvisors', title: 'Security', field: 'lints' },
      { op: 'getPerformanceAdvisors', title: 'Performance', field: 'lints' },
    ],
    actions: [],
  },
  {
    id: 'logs',
    label: 'Logs and usage',
    about: 'Recent logs across the project, and how many requests it served.',
    views: [{ op: 'getProjectUsageRequestCount', title: 'Requests' }],
    actions: [],
    link: { tab: 'logs', label: 'Logs from every resource are on the Logs tab' },
  },
  {
    id: 'billing',
    label: 'Compute and add-ons',
    about: 'The compute size and paid add-ons on this project.',
    views: [{ op: 'listProjectAddons', title: 'Add-ons' }],
    actions: [{ op: 'applyProjectAddon', label: 'Change compute or add-ons' }],
  },
];

export const supabase: ProviderEntry = {
  id: 'supabase',
  name: 'Supabase',
  facts: [
    {
      title: 'Project',
      op: 'getProject',
      on: ['database'],
      rows: [
        { label: 'Status', path: 'status' },
        { label: 'Postgres', path: 'database.version' },
        { label: 'Database host', path: 'database.host', as: 'code' },
        { label: 'Region', path: 'region', as: 'code' },
        { label: 'Created', path: 'created_at', as: 'ago' },
      ],
    },
    {
      title: 'Edge functions',
      op: 'listAllFunctions',
      on: ['database'],
      list: '',
      columns: [
        { label: 'Function', path: 'name' },
        { label: 'Status', path: 'status' },
        { label: 'Version', path: 'version' },
        { label: 'Updated', path: 'updated_at', as: 'ago' },
      ],
    },
    {
      title: 'Buckets',
      op: 'listAllBuckets',
      on: ['database'],
      list: '',
      columns: [
        { label: 'Bucket', path: 'name' },
        { label: 'Public', path: 'public', as: 'bool' },
        { label: 'Created', path: 'created_at', as: 'ago' },
      ],
    },
  ],
  logo: {
    colour: '#3FCF8E',
    path: 'M11.9 1.036c-.015-.986-1.26-1.41-1.874-.637L.764 12.05C-.33 13.427.65 15.455 2.409 15.455h9.579l.113 7.51c.014.985 1.259 1.408 1.873.636l9.262-11.653c1.093-1.375.113-3.403-1.645-3.403h-9.642z',
  },
  canImport: false,
  catalogue: { description: 'For apps built on Supabase', category: 'databases' },
  connect: {
    intro:
      'For apps already built on Supabase. Hoista sees your Supabase projects and connects your apps to them. Your data stays where it is.',
    token_steps: 'Account → Access Tokens → Generate new token. Name it Hoista.',
    token_label: 'Supabase access token',
    token_placeholder: 'sbp_…',
    sign_in: null,
    permissions: [],
  },
  accountId: {
    label: 'Organization ID',
    hint: 'The ID in your dashboard address: supabase.com/dashboard/org/<organization ID>.',
  },
  access: { read: 'Read', write: 'Write', delete: 'Write' },
  accountWord: 'Organization',
  groupWord: 'project',
  billing: 'https://supabase.com/dashboard/org/_/billing',
  dashboard: () => 'https://supabase.com/dashboard/projects',
  tile: { for: 'Database, auth and storage', group: 'databases' },
  pages: {
    colour: '#3ECF8E',
    signupOptions: ['Continue with GitHub', 'Sign up with email'],
    tokensNav: 'Access Tokens',
    tokensTitle: 'Access Tokens',
    createLabel: 'Generate new token',
    fields: [
      { label: 'Name', value: 'hoista' },
      { label: 'Access', value: 'Only the projects it needs' },
      { label: 'Expires', value: 'Up to 1 year' },
    ],
    tokenSteps: [
      'Open Account › Access Tokens',
      'Generate new token, name it hoista, choose all projects in your organization and set Projects to read-write, so Hoista can also create new ones',
      'Copy the token before closing the window',
    ],
    usedFor: 'Database, auth and storage',
    plan: 'Free plan pauses after a week idle',
  },
  words: {
    term: (kind, ref) =>
      kind === 'database' ? 'Postgres database' : ref.includes('/') ? 'Edge Function' : undefined,
    product: (kind) => (kind === 'database' ? 'Postgres' : ''),
    compute: 'Compute',
    storage: 'Storage',
    vcpu: false,
  },
  sections: SECTIONS,
  databaseLogs: true,
  // The project's secrets, which its edge functions read, live on its database.
  variables: { on: ['database'] },
  logKinds: ['bucket'],
  usageSeries: {
    op: 'getProjectUsageApiCount',
    field: 'result',
    intervals: { '1h': '1hr', '24h': '1day', '7d': '7day' },
    series: {
      total_rest_requests: 'Data API',
      total_auth_requests: 'Auth',
      total_storage_requests: 'Storage',
      total_realtime_requests: 'Realtime',
    },
    disk: 'getDiskUtilization',
  },
  fieldWords: {
    labels: {
      inserted_at: 'Created',
      created_at: 'Created',
      updated_at: 'Updated',
      postgres_engine: 'Postgres',
      release_channel: 'Release channel',
      organization_id: 'Organization',
      default_pool_size: 'Pool size',
      pool_mode: 'Pool mode',
      max_rows: 'Most rows per request',
      db_schema: 'Schemas served',
      verify_jwt: 'Needs a signed-in user',
      file_size_limit: 'Largest file',
      fileSizeLimit: 'Largest file',
      site_url: 'Site address',
      disable_signup: 'Sign-ups off',
      is_physical_backup: 'Full backup',
      inserted_at_unix: 'Created',
      dbAllowedCidrs: 'Allowed IPv4',
      dbAllowedCidrsV6: 'Allowed IPv6',
      currentConfig: 'Now',
      appliedSuccessfully: 'Applied',
      database_type: 'Database',
      db_port: 'Port',
      db_host: 'Host',
      db_name: 'Database name',
      db_user: 'User',
      ref: 'Project ref',
      pitr_enabled: 'Point-in-time restore',
      walg_enabled: 'Daily backups',
      schedule_for: 'Runs at',
      is_default: 'Main branch',
      event_message: 'Message',
      private_only: 'Private channels only',
      max_concurrent_users: 'Most people at once',
    },
    values: {
      ACTIVE_HEALTHY: 'Healthy',
      ACTIVE_UNHEALTHY: 'Unhealthy',
      INACTIVE: 'Paused',
      COMING_UP: 'Starting',
      PAUSING: 'Pausing',
      RESTORING: 'Resuming',
      RESTARTING: 'Restarting',
      ACTIVE: 'Active',
      THROTTLED: 'Throttled',
      COMPLETED: 'Done',
      transaction: 'Transaction',
      session: 'Session',
      PRIMARY: 'Primary',
      db: 'Database',
      rest: 'Data API',
      auth: 'Auth',
      realtime: 'Realtime',
      storage: 'Storage',
      pooler: 'Connection pooler',
      in_use: 'In use',
      standby: 'Standby',
      revoked: 'Revoked',
      previously_used: 'Used before',
    },
    units: {},
  },
  console: (reference) => {
    const [project] = reference.split('/');
    return project
      ? `https://supabase.com/dashboard/project/${project}`
      : 'https://supabase.com/dashboard/projects';
  },
  // `abcd.supabase.co`, `db.abcd.supabase.co`, or the pooler's user `postgres.abcd`.
  owns: ({ host, user }) => {
    const direct = /^(?:db\.)?([a-z0-9]{20})\.supabase\.(?:co|com)$/.exec(host);
    const pooled = host.endsWith('.pooler.supabase.com')
      ? /^postgres\.([a-z0-9]{20})$/.exec(user ?? '')
      : null;
    const ref = direct?.[1] ?? pooled?.[1];
    return ref ? `supabase:${ref}` : null;
  },
  // The project is the group: its database, functions and buckets.
  project: (reference) => {
    const [project] = reference.split('/');
    return project ? `supabase:${project}` : null;
  },
  usedVia: /^(?:NEXT_PUBLIC_|VITE_|PUBLIC_|EXPO_PUBLIC_)?SUPABASE_/,
};
