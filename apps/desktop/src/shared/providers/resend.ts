/** Resend. */
import type { ProviderEntry, ProviderSection } from './types';

/** Resend’s tab: the sections each of its resources gets. */
const SECTIONS: ProviderSection[] = [
  {
    id: 'overview',
    label: 'Overview',
    about: 'This sending domain: its DNS records and whether each is verified.',
    views: [{ op: 'domainsGet', title: 'Domain', about: 'Records to add at your DNS host' }],
    actions: [
      { op: 'domainsVerify', label: 'Check DNS again' },
      { op: 'domainsUpdate', label: 'Change settings' },
    ],
  },
  {
    id: 'emails',
    label: 'Emails',
    about: 'The team’s latest emails and what happened to each.',
    views: [{ op: 'emailsList', title: 'Recent emails', field: 'data', args: { limit: 20 } }],
    actions: [],
  },
];

export const resend: ProviderEntry = {
  id: 'resend',
  name: 'Resend',
  facts: [
    {
      title: 'Domain',
      op: 'domainsGet',
      rows: [
        { label: 'Status', path: 'status' },
        { label: 'Region', path: 'region', as: 'code' },
        { label: 'Open tracking', path: 'open_tracking', as: 'bool' },
        { label: 'Click tracking', path: 'click_tracking', as: 'bool' },
        { label: 'TLS', path: 'tls' },
        { label: 'Created', path: 'created_at', as: 'ago' },
      ],
    },
    {
      title: 'DNS records',
      op: 'domainsGet',
      list: 'records',
      columns: [
        { label: 'Record', path: 'record' },
        { label: 'Type', path: 'type', as: 'code' },
        { label: 'Name', path: 'name', as: 'code' },
        { label: 'Status', path: 'status' },
      ],
    },
  ],
  charts: {},
  // A sending domain's logs are the emails sent from it.
  logKinds: ['email'],
  logo: {
    colour: 'var(--text)',
    path: 'M14.679 0c4.648 0 7.413 2.765 7.413 6.434s-2.765 6.434-7.413 6.434H12.33L24 24h-8.245l-8.88-8.44c-.636-.588-.93-1.273-.93-1.86 0-.831.587-1.565 1.713-1.883l4.574-1.224c1.737-.465 2.936-1.81 2.936-3.572 0-2.153-1.761-3.4-3.939-3.4H0V0z',
  },
  canImport: false,
  catalogue: { description: 'Email for your apps', category: 'email' },
  connect: {
    intro:
      'Your sending domains, emails and contacts. Hoista shows each domain and whether it can send yet.',
    token_steps: 'API Keys → Create API Key → Full access. Name it Hoista.',
    token_label: 'Resend API key',
    token_placeholder: 're_…',
    sign_in: null,
    permissions: [],
  },
  pages: {
    colour: '#111111',
    signupOptions: ['Continue with GitHub', 'Continue with email'],
    tokensNav: 'API Keys',
    tokensTitle: 'API Keys',
    createLabel: 'Create API key',
    fields: [
      { label: 'Name', value: 'hoista' },
      { label: 'Permission', value: 'Full access' },
      { label: 'Domain', value: 'All domains' },
    ],
    tokenSteps: [
      'Open API Keys in the sidebar',
      'Name it hoista and give it full access',
      'Click Create API key, then copy it. It’s shown once.',
    ],
    usedFor: 'Sign-up and password emails',
    plan: '3,000 emails a month free, up to 100 a day',
  },
  words: {
    term: () => 'Sending domain',
    product: () => 'Domain',
    compute: 'Compute',
    storage: 'Storage',
    vcpu: false,
  },
  console: (reference) => `https://resend.com/domains/${encodeURIComponent(reference)}`,
  sampleToken: 're_••••••••••d42b',
  projectsWord: 'Domains',
  sections: SECTIONS,
  access: { read: 'Full access', write: 'Full access', delete: 'Full access' },
  accountWord: 'Team',
  billing: 'https://resend.com/settings/billing',
  dashboard: () => 'https://resend.com/overview',
  tile: { for: 'Sending domains and email', group: 'email' },
  usedVia: /^RESEND_/,
};
