/** Runpod. */
import type { ProviderEntry, ProviderSection } from './types';

/**
 * Runpod's tab, by what the resource is: a pod (`pod:`), a Serverless
 * endpoint (`endpoint:`) or a network volume (`volume:`). Operations come
 * from Runpod's REST API v2.
 */
const SECTIONS: ProviderSection[] = [
  {
    id: 'overview',
    label: 'Overview',
    about: 'This pod on Runpod: its GPU, image, ports and what it costs an hour.',
    refs: ['pod:'],
    views: [{ op: 'getPod', title: 'Pod' }],
    actions: [
      { op: 'podAction', label: 'Restart', args: { action: 'restart' } },
      { op: 'podAction', label: 'Stop', args: { action: 'stop' } },
      { op: 'podAction', label: 'Start', args: { action: 'start' } },
      { op: 'updatePod', label: 'Change settings' },
    ],
  },
  {
    id: 'billing',
    label: 'Billing',
    about: 'What this pod has cost, day by day, as Runpod bills it.',
    refs: ['pod:'],
    views: [
      { op: 'listPodBilling', title: 'Spend', args: { bucketSize: 'day' }, empty: 'No spend yet.' },
    ],
    actions: [],
  },
  {
    id: 'overview',
    label: 'Overview',
    about: 'This Serverless endpoint: its GPU pools, scaling and addresses.',
    refs: ['endpoint:'],
    views: [{ op: 'getEndpoint', title: 'Endpoint' }],
    actions: [{ op: 'updateEndpoint', label: 'Change settings' }],
  },
  {
    id: 'workers',
    label: 'Workers',
    about: 'The endpoint’s workers right now and what each is doing.',
    refs: ['endpoint:'],
    views: [{ op: 'listEndpointWorkers', title: 'Workers', empty: 'No workers running.' }],
    actions: [],
  },
  {
    id: 'releases',
    label: 'Releases and builds',
    about: 'Each release of the endpoint, and the builds behind them.',
    refs: ['endpoint:'],
    views: [
      { op: 'listEndpointReleases', title: 'Releases', empty: 'No releases yet.' },
      { op: 'listEndpointBuilds', title: 'Builds', empty: 'No builds yet.' },
    ],
    actions: [],
  },
  {
    id: 'billing',
    label: 'Billing',
    about: 'What this endpoint has cost, day by day, as Runpod bills it.',
    refs: ['endpoint:'],
    views: [
      {
        op: 'listServerlessBilling',
        title: 'Spend',
        args: { bucketSize: 'day' },
        empty: 'No spend yet.',
      },
    ],
    actions: [],
  },
  {
    id: 'overview',
    label: 'Overview',
    about: 'This network volume: its size, data center and type.',
    refs: ['volume:'],
    views: [{ op: 'getNetworkVolume', title: 'Volume' }],
    actions: [{ op: 'updateNetworkVolume', label: 'Resize or rename' }],
  },
  {
    id: 'billing',
    label: 'Billing',
    about: 'What this volume has cost, day by day, as Runpod bills it.',
    refs: ['volume:'],
    views: [
      {
        op: 'listNetworkVolumeBilling',
        title: 'Spend',
        args: { bucketSize: 'day' },
        empty: 'No spend yet.',
      },
    ],
    actions: [],
  },
];

/** The id inside a `pod:…`, `endpoint:…` or `volume:…` reference. */
function idOf(reference: string): string {
  return encodeURIComponent(reference.slice(reference.indexOf(':') + 1));
}

export const runpod: ProviderEntry = {
  id: 'runpod',
  name: 'Runpod',
  facts: [
    {
      title: 'Pod',
      op: 'getPod',
      on: ['service'],
      refs: ['pod:'],
      rows: [
        { label: 'Status', path: 'status' },
        { label: 'GPU', path: 'gpu.type' },
        { label: 'GPUs', path: 'gpu.count' },
        { label: 'Image', path: 'image', as: 'code' },
        { label: 'Data center', path: 'dataCenterId', as: 'code' },
        { label: 'Cloud', path: 'cloud' },
        { label: 'Cost an hour', path: 'cost' },
        { label: 'Started', path: 'startedAt', as: 'ago' },
      ],
    },
  ],
  charts: {},
  logo: {
    colour: '#673ab7',
    fillRule: 'evenodd',
    path: 'M21.6 19.4C22.8 18.8 23.6 17.6 23.6 16.2L23.6 7.8C23.6 6.4 22.8 5.2 21.6 4.6L14.4 0.8C13.2 0.2 11.8 0.2 10.6 0.8L3.4 4.6C2.2 5.2 1.4 6.4 1.4 7.8L1.4 16.2C1.4 17.6 2.2 18.8 3.4 19.4L10.6 23.2C11.8 23.8 13.2 23.8 14.4 23.2L21.6 19.4ZM21.6 16.2C21.6 16.8 21.2 17.4 20.6 17.7L17.2 19.2C16.8 19.4 16.6 19.5 16.4 19.5C16.2 19.5 16.1 19.4 16 19.2C15.9 19 15.9 18.6 15.9 17.8L15.9 15.2C15.9 14.6 16.3 14 16.9 13.7L19.3 12.2C19.7 12 19.9 11.9 20 11.8C20.1 11.7 20.2 11.6 20.2 11.4C20.3 11.2 20.3 11 20.3 10.6L20.3 10.4C20.3 9.8 20.3 9.5 20.1 9.3C20 9.1 19.8 9 19.6 9C19.4 9 19.2 9.2 18.8 9.6L15.9 11.2C14.7 11.8 13.3 12.2 13.3 15.2V21.2C13.3 21.6 13.1 21.9 12.8 22C12.2 22.3 11.8 22.3 11.2 22L4 19.4C3.4 19.1 3 18.5 3 17.9L3 11.9C3 11.3 3 10.9 3.1 10.7C3.2 10.5 3.4 10.4 3.6 10.4C3.8 10.4 4 10.5 4.2 10.7L7.4 12.2C8.2 12.5 8.6 13.1 8.6 13.7L8.6 15.7C8.6 16.1 8.6 16.3 8.7 16.5C8.8 16.7 8.9 16.8 9 16.9C9.1 17 9.2 17.1 9.4 17.2L9.5 17.2C9.8 17.4 9.9 17.5 10 17.5C10.1 17.5 10.2 17.4 10.3 17.2C10.4 17 10.4 16.6 10.4 15.8L10.4 13.7C10.4 12.3 9.6 11.1 8.4 10.5L2.8 7.8C2.4 7.6 2 7.2 2 6.8C2 6.2 2.4 5.6 3 5.3L11.2 1.5C12 1.2 12.8 1.2 13.6 1.5L17.4 3.2C17.8 3.4 18 3.5 18.1 3.7C18.2 3.8 18.2 4 18.1 4.2C18 4.4 17.8 4.5 17.4 4.7L13.6 6.4C12.8 6.7 12 6.7 11.2 6.4L8.8 5.2C8.4 5 8.2 4.9 8 4.8C7.8 4.8 7.6 4.8 7.4 4.9C7.2 5 7 5.1 6.8 5.3L6.4 5.5C6 5.7 5.8 5.8 5.7 6C5.6 6.1 5.6 6.3 5.7 6.5C5.8 6.7 6 6.8 6.4 7L9.6 8.5C10.8 9.1 12.2 9.1 13.4 8.5L20.2 5.3C20.6 5.1 21 5.1 21.4 5.3C22 5.6 22.4 6.2 22.4 6.8V16.2Z',
  },
  canImport: true,
  catalogue: { description: 'GPU pods and Serverless endpoints', category: 'gpu' },
  connect: {
    intro:
      'Your GPU and CPU pods, Serverless endpoints and network volumes. Hoista shows each, what it’s doing and what it has cost.',
    token_steps: 'Settings → API Keys → Create API Key → All. Name it Hoista.',
    token_label: 'Runpod API key',
    token_placeholder: 'rpa_…',
    sign_in: null,
    permissions: [],
  },
  pages: {
    colour: '#673ab7',
    signupOptions: ['Continue with Google', 'Continue with GitHub', 'Sign up with email'],
    tokensNav: 'Settings',
    tokensTitle: 'API Keys',
    createLabel: 'Create API Key',
    fields: [
      { label: 'Name', value: 'hoista' },
      { label: 'Permissions', value: 'All' },
    ],
    tokenSteps: [
      'Open Settings › API Keys in the console',
      'Click Create API Key, name it hoista and choose All',
      'Copy the key. It’s shown once.',
    ],
    usedFor: 'GPU pods and Serverless endpoints',
    plan: 'Pay by the second; no free tier',
    headsUp: 'A Read only key lets Hoista see your pods but not restart or stop them.',
  },
  words: {
    term: (kind) => (kind === 'bucket' ? 'Network volume' : 'Pod'),
    product: (kind) => (kind === 'bucket' ? 'Network volume' : 'Pod'),
    compute: 'GPU',
    storage: 'Volume',
    vcpu: false,
  },
  console: (reference) =>
    reference.startsWith('pod:')
      ? `https://console.runpod.io/pods?id=${idOf(reference)}`
      : reference.startsWith('endpoint:')
        ? `https://console.runpod.io/serverless/user/endpoint/${idOf(reference)}`
        : 'https://console.runpod.io/user/storage',
  sampleToken: 'rpa_••••••••••d42b',
  projectsWord: 'Pods, endpoints and volumes',
  sections: SECTIONS,
  access: { read: 'Read only', write: 'All', delete: 'All' },
  accountWord: 'Account',
  billing: 'https://console.runpod.io/user/billing',
  dashboard: () => 'https://console.runpod.io',
  fieldWords: {
    labels: {
      dataCenterId: 'Data center',
      cudaVersion: 'CUDA',
      vcpuCount: 'vCPUs',
      totalAmount: 'Total (USD)',
      podGpuAmount: 'Pod GPU (USD)',
      podCpuAmount: 'Pod CPU (USD)',
      podDiskAmount: 'Pod disk (USD)',
      serverlessGpuAmount: 'Serverless GPU (USD)',
      serverlessCpuAmount: 'Serverless CPU (USD)',
      serverlessDiskAmount: 'Serverless disk (USD)',
      startTime: 'From',
      endTime: 'To',
      flashboot: 'FlashBoot',
    },
    values: {
      RUNNING: 'Running',
      EXITED: 'Stopped',
      PROVISIONING: 'Provisioning',
      SECURE: 'Secure Cloud',
      COMMUNITY: 'Community Cloud',
      QUEUE: 'Queue',
      LOAD_BALANCER: 'Load balancer',
    },
    units: { disk: 'GB', size: 'GB', memory: 'GB' },
  },
  tile: { for: 'GPU pods, Serverless endpoints and volumes', group: 'gpu' },
};
