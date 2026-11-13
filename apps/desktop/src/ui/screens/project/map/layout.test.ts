import { describe, expect, it } from 'vitest';
import type { Link, ProjectMap, Resource } from '@shared/api';
import {
  CANVAS_HEIGHT,
  CANVAS_WIDTH,
  NODE_HEIGHT,
  NODE_WIDTH,
  clampToCanvas,
  canvasHeight,
  MOST_HEIGHT,
  layoutMap,
} from './layout';

const resource = (id: string, kind: Resource['kind'], name: string): Resource => ({
  id,
  project_id: 'moodboard',
  kind,
  provider: 'cloudflare',
  product: '',
  name,
  region: null,
  status: 'live',
  provider_ref: name,
  usage: null,
});

/** moodboard: a site and its nightly job, both using a database and a bucket. */
const links: Link[] = [
  { from: 'moodboard-web', to: 'moodboard-db', via: 'DATABASE_URL' },
  { from: 'moodboard-web', to: 'moodboard-uploads', via: 'UPLOADS' },
  { from: 'moodboard-digest', to: 'moodboard-db', via: 'DATABASE_URL' },
];
const map: ProjectMap = {
  nodes: [
    resource('moodboard-web', 'web', 'moodboard'),
    resource('moodboard-digest', 'job', 'nightly-digest'),
    resource('moodboard-db', 'database', 'moodboard-db'),
    resource('moodboard-uploads', 'bucket', 'moodboard-uploads'),
  ],
  edges: links,
  traffic: {},
};
const project = {
  source: '~/code/moodboard',
  sourceDetail: 'your folder',
  url: 'moodboard.sam.workers.dev',
  urlDetail: '12,480 requests · 7d',
};

describe('layoutMap', () => {
  it('puts runtime resources in the middle column and data on the right', () => {
    const { nodes } = layoutMap(project, map);
    const column = (id: string) => nodes.find((node) => node.id === id)?.x;

    expect(column('source')).toBe(0);
    expect(column('moodboard-web')).toBe(column('moodboard-digest'));
    expect(column('moodboard-db')).toBeGreaterThan(column('moodboard-web') ?? 0);
  });

  it('puts a domain with what comes in, not with the data', () => {
    const [first] = map.nodes;
    if (!first) throw new Error('fixture has resources');
    const withDomain: ProjectMap = {
      ...map,
      nodes: [...map.nodes, { ...first, id: 'zone', kind: 'domain', name: 'sam.dev' }],
    };
    const { nodes } = layoutMap(project, withDomain);
    expect(nodes.find((node) => node.id === 'zone')?.x).toBe(0);
  });

  it('keeps every node on the canvas', () => {
    for (const node of layoutMap(project, map).nodes) {
      expect(node.y).toBeGreaterThanOrEqual(0);
      expect(node.y + NODE_HEIGHT).toBeLessThanOrEqual(CANVAS_HEIGHT);
    }
  });

  it('grows the canvas for a long column, without boxes overlapping', () => {
    const first = map.nodes[0];
    if (!first) throw new Error('fixture');
    const many = {
      ...map,
      nodes: Array.from({ length: 10 }, (_, index) => ({
        ...first,
        id: `web-${String(index)}`,
        kind: 'web' as const,
      })),
      edges: [],
    };
    const { nodes } = layoutMap(project, many);
    const column = nodes.filter((node) => node.id.startsWith('web-')).map((node) => node.y);
    for (let index = 1; index < column.length; index += 1) {
      expect((column[index] ?? 0) - (column[index - 1] ?? 0)).toBeGreaterThanOrEqual(NODE_HEIGHT);
    }
    const height = canvasHeight(nodes);
    expect(height).toBeGreaterThan(CANVAS_HEIGHT);
    for (const node of nodes) expect(node.y + NODE_HEIGHT).toBeLessThanOrEqual(height);
  });

  it('puts a box added after arranging below the arranged ones, and moves nothing else', () => {
    const before = layoutMap(project, map).nodes;
    const arranged = Object.fromEntries(
      before.map((node) => [node.id, { x: node.x + 5, y: node.y + 200 }]),
    );
    const first = map.nodes.find((node) => node.kind === 'web') ?? map.nodes[0];
    if (!first) throw new Error('fixture');
    const added = { ...map, nodes: [...map.nodes, { ...first, id: 'new-web' }] };
    const after = layoutMap(project, added, arranged).nodes;
    for (const node of before) {
      expect(after.find((other) => other.id === node.id)).toMatchObject(arranged[node.id] ?? {});
    }
    const column = before.filter((node) => node.x === after.find((n) => n.id === 'new-web')?.x);
    const fresh = after.find((node) => node.id === 'new-web');
    for (const node of column) {
      expect(fresh?.y ?? 0).toBeGreaterThanOrEqual((arranged[node.id]?.y ?? 0) + NODE_HEIGHT);
    }
  });

  it('gives each other repository its own box, and each service its own line', () => {
    const web = map.nodes.find((node) => node.kind === 'web') ?? map.nodes[0];
    if (!web) throw new Error('fixture');
    const many = {
      ...map,
      nodes: [
        ...map.nodes,
        { ...web, id: 'dashboard', repo: 'sam/dashboard' },
        { ...web, id: 'same', repo: 'sam/project' },
      ],
    };
    const { nodes, edges } = layoutMap({ ...project, sourceRepo: 'sam/project' }, many);
    expect(nodes.find((node) => node.id === 'repo:sam/dashboard')?.title).toBe('sam/dashboard');
    expect(nodes.some((node) => node.id === 'repo:sam/project')).toBe(false);
    const ids = edges.map((edge) => edge.id);
    expect(ids).toContain('repo:sam/dashboard>dashboard');
    expect(ids).toContain('source>same');
    expect(ids).not.toContain('source>dashboard');
  });

  it('draws deploy, route and link edges', () => {
    const labels = layoutMap(project, map).edges.map((edge) => edge.label);

    expect(labels).toContain('deploys');
    expect(labels).toContain('route');
    expect(labels).toContain('DATABASE_URL');
  });

  it('leads the address to the service when the provider calls everything a service', () => {
    const [first] = map.nodes;
    if (!first) throw new Error('fixture has resources');
    const railway: ProjectMap = {
      nodes: [{ ...first, id: 'api', kind: 'service' }],
      edges: [],
      traffic: {},
    };
    const route = layoutMap(project, railway).edges.find((edge) => edge.label === 'route');

    expect(route?.id).toBe('address>api');
  });

  it('leads the address only to the resource the provider serves it from', () => {
    const [first] = map.nodes;
    if (!first) throw new Error('fixture has resources');
    const railway: ProjectMap = {
      nodes: [
        { ...first, id: 'api', kind: 'service', url: 'https://moodboard.sam.workers.dev' },
        { ...first, id: 'worker', kind: 'service', url: null },
      ],
      edges: [],
      traffic: {},
    };
    const routes = layoutMap(project, railway).edges.filter((edge) => edge.label === 'route');

    expect(routes.map((edge) => edge.id)).toEqual(['address>api']);
  });

  it('keeps a dragged box where it was dropped, with its edges following', () => {
    const { nodes, edges } = layoutMap(project, map, { 'moodboard-db': { x: 400, y: 300 } });
    const db = nodes.find((node) => node.id === 'moodboard-db');
    const edge = edges.find((candidate) => candidate.id.endsWith('>moodboard-db'));

    expect(db).toMatchObject({ x: 400, y: 300 });
    expect(edge?.path.endsWith(`${300 + NODE_HEIGHT / 2}`)).toBe(true);
  });

  it('never lets a box be dragged off the canvas', () => {
    expect(clampToCanvas(-50, 9999)).toEqual({ x: 0, y: MOST_HEIGHT - NODE_HEIGHT });
    expect(clampToCanvas(9999, 100).x).toBe(CANVAS_WIDTH - NODE_WIDTH);
  });
});
