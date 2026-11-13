/** Fly.io. */
import type { ProviderEntry, ProviderSection } from './types';

/** Fly’s tab: the sections each of its resources gets. */
const SECTIONS: ProviderSection[] = [
  {
    id: 'overview',
    label: 'Overview',
    about: 'This app on Fly.io: its status, Machines and where they run.',
    views: [
      { op: 'appsShow', title: 'App' },
      { op: 'fly status', title: 'Status', about: 'Machines, regions and health' },
    ],
    actions: [],
  },
  {
    id: 'machines',
    label: 'Machines',
    about: 'The Machines that run this app, and how it’s scaled.',
    views: [
      { op: 'machinesList', title: 'Machines' },
      { op: 'fly scale show', title: 'Scale', about: 'Machines per process and their size' },
    ],
    actions: [{ op: 'machinesCreate', label: 'New Machine' }],
  },
  {
    id: 'releases',
    label: 'Releases',
    about: 'Every release of this app, newest first.',
    views: [{ op: 'fly releases', title: 'Releases' }],
    actions: [],
  },
  {
    id: 'networking',
    label: 'Networking',
    about: 'The app’s IP addresses and TLS certificates.',
    views: [
      { op: 'fly ips list', title: 'IP addresses' },
      { op: 'appCertificatesList', title: 'Certificates' },
    ],
    actions: [{ op: 'appCertificatesAcmeCreate', label: 'Add a certificate' }],
  },
  {
    id: 'volumes',
    label: 'Volumes',
    about: 'Disks attached to this app’s Machines.',
    views: [{ op: 'volumesList', title: 'Volumes' }],
    actions: [{ op: 'volumesCreate', label: 'New volume' }],
  },
  {
    id: 'checks',
    label: 'Health checks',
    about: 'The checks Fly.io runs against this app.',
    views: [{ op: 'fly checks list', title: 'Checks' }],
    actions: [],
  },
];

export const fly: ProviderEntry = {
  id: 'fly',
  name: 'Fly.io',
  facts: [
    {
      title: 'App',
      op: 'appsShow',
      rows: [
        { label: 'Status', path: 'status' },
        { label: 'Organization', path: 'organization.slug', as: 'code' },
      ],
    },
    {
      title: 'Machines',
      op: 'machinesList',
      list: '',
      columns: [
        { label: 'Machine', path: 'id', as: 'code' },
        { label: 'Region', path: 'region', as: 'code' },
        { label: 'State', path: 'state' },
        { label: 'Memory', path: 'config.guest.memory_mb', as: 'mb' },
        { label: 'Updated', path: 'updated_at', as: 'ago' },
      ],
    },
    {
      title: 'Volumes',
      op: 'volumesList',
      list: '',
      columns: [
        { label: 'Volume', path: 'name' },
        { label: 'Region', path: 'region', as: 'code' },
        { label: 'Size', path: 'size_gb', as: 'gb' },
        { label: 'State', path: 'state' },
      ],
    },
  ],
  charts: {},
  logo: {
    colour: '#A78BFA',
    path: 'M11.987 0c-2.45-.01-5.002.925-6.541 2.897-1.17 1.502-1.664 3.474-1.49 5.356.29 2.112 1.476 3.96 2.676 5.672a41.5 41.5 0 0 0 4.216 4.831c-1.063.832-1.943 2.286-1.357 3.644.821 2.32 4.665 2.05 5.122-.372.39-1.288-.694-2.533-1.428-3.309 2.388-2.431 4.706-5.036 6.17-8.145.595-1.32.902-2.802.614-4.24-.28-2.341-1.823-4.473-3.967-5.46C14.76.266 13.364.016 11.987 0m-.236 1.577v15.534C9.881 13.483 7.724 9.266 8.73 5.069c.35-1.539 1.253-3.309 3.02-3.492m1.996.04c1.534.357 3.031 1.096 3.906 2.48 1.3 1.93 1.318 4.55.1 6.521-1.268 2.395-3.06 4.463-4.916 6.415 1.472-2.974 3.074-6.106 3.182-9.5-.043-2.08-.438-4.612-2.272-5.916M11.97 20.103c.848.342 1.597 1.983.153 2.173-.664.15-1.367-.599-.995-1.222.213-.355.488-.73.842-.95',
  },
  canImport: false,
  catalogue: { description: 'App servers that sleep when idle', category: 'hosting' },
  connect: {
    intro: 'Runs app servers that stop when nobody is visiting.',
    token_steps:
      'Pick your organization → Tokens → Create Organization Token. Fly asks for a card to prevent abuse.',
    token_label: 'Fly.io token',
    token_placeholder: 'FlyV1 …',
    sign_in: null,
    permissions: [],
  },
  accountId: {
    label: 'Organization slug',
    hint: 'The name in your Fly dashboard address, for example personal.',
  },
  accountWord: 'Organization',
  groupWord: 'app',
  billing: 'https://fly.io/dashboard/personal/billing',
  dashboard: () => 'https://fly.io/dashboard',
  tile: { for: 'Servers that sleep when idle', group: 'hosting' },
  pages: {
    colour: '#7B3BE2',
    signupOptions: ['Sign up with GitHub', 'Sign up with email'],
    tokensNav: 'Tokens',
    tokensTitle: 'Organization Tokens',
    createLabel: 'Create Organization Token',
    fields: [
      { label: 'Token name', value: 'hoista' },
      { label: 'Expiration', value: 'Optional, like 8760h for a year' },
    ],
    tokenSteps: [
      'Pick your organization, then open Tokens',
      'Name it hoista. Leave expiration empty, or set one like 8760h',
      'Click Create Organization Token, then copy it',
    ],
    usedFor: 'Servers that sleep when idle',
    plan: 'Needs a card before you can deploy',
    headsUp: 'Fly.io asks for a payment method before it runs any app, even on the trial.',
  },
  words: {
    term: () => 'App',
    product: () => 'Machines',
    compute: 'Compute',
    storage: 'Storage',
    vcpu: false,
  },
  console: (reference) =>
    reference ? `https://fly.io/apps/${encodeURIComponent(reference)}` : 'https://fly.io/dashboard',
  sampleToken: 'fo1_•••••••••••7c0e',
  projectsWord: 'Apps',
  sections: SECTIONS,
};
