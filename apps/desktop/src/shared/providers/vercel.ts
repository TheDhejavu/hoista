/** Vercel. */
import type { ProviderEntry, ProviderSection } from './types';

/** Vercel’s tab: the sections each of its resources gets. */
const SECTIONS: ProviderSection[] = [
  {
    id: 'overview',
    label: 'Overview',
    about: 'This project on Vercel: its framework, build settings and production branch.',
    views: [
      {
        op: 'getProject',
        title: 'Project',
        about: 'Framework, build and output settings, Node version',
      },
    ],
    actions: [
      { op: 'updateProject', label: 'Change settings' },
      { op: 'pauseProject', label: 'Pause' },
      { op: 'unpauseProject', label: 'Resume' },
    ],
  },
  {
    id: 'deployments',
    label: 'Deployments',
    about: 'Every deployment of this project, and rolling releases.',
    views: [
      {
        op: 'getDeployments',
        title: 'Deployments',
        about: 'Newest first',
        field: 'deployments',
        args: { limit: 20 },
      },
      { op: 'getRollingRelease', title: 'Rolling release', about: 'A release going out in stages' },
    ],
    actions: [{ op: 'updateRollingReleaseConfig', label: 'Rolling release settings' }],
  },
  {
    id: 'domains',
    label: 'Domains',
    about: 'The project’s domains and the aliases pointing at its deployments.',
    views: [
      { op: 'getProjectDomains', title: 'Domains', field: 'domains' },
      { op: 'listAliases', title: 'Aliases', field: 'aliases' },
    ],
    actions: [],
  },
  {
    id: 'routing',
    label: 'Redirects and routes',
    about: 'Redirects and routing rules Vercel applies before your code.',
    views: [
      { op: 'getRedirects', title: 'Redirects' },
      { op: 'getRoutes', title: 'Routes' },
    ],
    actions: [{ op: 'stageRedirects', label: 'Change redirects' }],
  },
  {
    id: 'environments',
    label: 'Environments',
    about: 'Production, Preview, Development and custom environments.',
    views: [{ op: 'getProjectsByIdOrNameCustomEnvironments', title: 'Custom environments' }],
    actions: [{ op: 'createCustomEnvironment', label: 'New environment' }],
  },
  {
    id: 'security',
    label: 'Firewall',
    about: 'Attack mode and what Vercel’s firewall blocked.',
    views: [
      { op: 'getActiveAttackStatus', title: 'Attack mode' },
      { op: 'getSecurityFirewallEvents', title: 'Firewall events' },
    ],
    actions: [],
  },
  {
    id: 'drains',
    label: 'Log drains',
    about: 'Where Vercel sends this project’s logs.',
    views: [{ op: 'getDrains', title: 'Drains' }],
    actions: [],
  },
];

export const vercel: ProviderEntry = {
  id: 'vercel',
  name: 'Vercel',
  facts: [
    {
      title: 'Project',
      op: 'getProject',
      rows: [
        { label: 'Framework', path: 'framework' },
        { label: 'Node.js', path: 'nodeVersion' },
        { label: 'Repository', path: 'link.repo', as: 'code' },
        { label: 'Production branch', path: 'link.productionBranch', as: 'code' },
        { label: 'Root directory', path: 'rootDirectory', as: 'code' },
        { label: 'Created', path: 'createdAt', as: 'ago' },
      ],
    },
    {
      title: 'Domains',
      op: 'getProjectDomains',
      list: 'domains',
      columns: [
        { label: 'Domain', path: 'name', as: 'code' },
        { label: 'Verified', path: 'verified', as: 'bool' },
        { label: 'Redirects to', path: 'redirect', as: 'code' },
      ],
    },
  ],
  charts: { plan: 'https://vercel.com/docs/observability/observability-plus' },
  logo: {
    colour: 'var(--text)',
    path: 'M24 22.525H0l12-21.05 12 21.05z',
  },
  canImport: true,
  catalogue: { description: 'Import your existing projects', category: 'hosting' },
  connect: {
    intro: 'Hoista sees your Vercel projects so you can watch or manage them here.',
    token_steps: 'Account Settings → Tokens → Create. Name it Hoista.',
    token_label: 'Vercel token',
    token_placeholder: 'Paste your Vercel token',
    sign_in: { on_their_page: 'You pick the team and what Hoista may do there.' },
    permissions: [],
  },
  accountId: { label: 'Team ID', hint: 'In Vercel: Team Settings → General → Team ID.' },
  accountWord: 'Team',
  groupWord: 'project',
  billing: 'https://vercel.com/~/settings/billing',
  dashboard: () => 'https://vercel.com/dashboard',
  accountFacts: [
    {
      title: 'Team',
      op: 'getTeam',
      rows: [
        { label: 'Name', path: 'name' },
        { label: 'URL name', path: 'slug', as: 'code' },
        { label: 'Plan', path: 'billing.plan' },
        { label: 'Your role', path: 'membership.role' },
        { label: 'Created', path: 'createdAt', as: 'ago' },
      ],
    },
  ],
  tile: { for: 'Frontends and functions', group: 'hosting' },
  pages: {
    colour: '#111111',
    signupOptions: ['Continue with GitHub', 'Continue with email'],
    tokensNav: 'Tokens',
    tokensTitle: 'Tokens',
    createLabel: 'Create',
    fields: [
      { label: 'Token name', value: 'hoista' },
      { label: 'Scope', value: 'Your team, or one project' },
      { label: 'Expiration', value: '1 Year' },
    ],
    tokenSteps: [
      'Open Account Settings › Tokens',
      'Name it hoista and pick the team or the one project it can reach',
      'Click Create, then copy the token. It’s shown once.',
    ],
    usedFor: 'Frontends and functions',
    plan: 'Hobby plan is free',
  },
  words: {
    term: () => 'Project',
    product: () => 'Project',
    compute: 'Compute',
    storage: 'Storage',
    vcpu: false,
  },
  console: () => 'https://vercel.com/dashboard',
  // `https://vercel.com/{team}/{project}/{deployment}` → the project's page.
  consoleFrom: (page) =>
    /^https:\/\/vercel\.com\/[^/]+\/[^/]+\/[^/]+$/.test(page) ? page.replace(/\/[^/]+$/, '') : null,
  sections: SECTIONS,
  domainActions: { add: true, remove: true },
};
