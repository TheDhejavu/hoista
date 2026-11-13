/**
 * Lays out the app map: three columns (what comes in, what runs the code,
 * what keeps the data), nodes spread evenly down each column, and curved
 * edges between them. Boxes the user dragged keep where they were dropped.
 * Pure, so it can be tested without a browser.
 */
import type { ProjectMap, Resource, ResourceKind, ProviderId, Status } from '@shared/api';

export const NODE_WIDTH = 210;
export const NODE_HEIGHT = 62;
/** The canvas's least height; it grows with the boxes in it. */
export const CANVAS_HEIGHT = 430;
/** How far down a box can be dragged, which makes room below the rest. */
export const MOST_HEIGHT = 4000;
// Each box's share of a column, at least: a column of ten doesn't overlap.
const ROW = NODE_HEIGHT + 30;
// Room under the lowest box.
const BOTTOM = 40;
// Wide enough for an edge label such as DATABASE_URL to sit between two boxes.
const COLUMN_GAP = 112;
const TOP = 40;

export const COLUMN_TITLES = ['Source and domains', 'Services', 'Data and dependencies'] as const;

export const CANVAS_WIDTH =
  COLUMN_TITLES.length * NODE_WIDTH + (COLUMN_TITLES.length - 1) * COLUMN_GAP;

/** The left edge of a column. */
export function columnX(column: number): number {
  return column * (NODE_WIDTH + COLUMN_GAP);
}

/** A box on the map. Only `resource` nodes open details when clicked. */
export interface MapNode {
  id: string;
  kind: 'source' | 'address' | 'resource';
  title: string;
  detail: string;
  status: Status;
  provider: ProviderId | null;
  /** A logo other than a provider's, such as GitHub's for a linked repository. */
  icon?: string;
  /** Where clicking a box that isn't a resource goes: the repository or the address. */
  href?: string;
  x: number;
  y: number;
}

/** A line between two boxes, with its label at the midpoint. */
export interface MapEdge {
  id: string;
  path: string;
  label: string;
  labelX: number;
  labelY: number;
  /** Dashed lines show where code is deployed from; solid ones carry traffic. */
  style: 'deploys' | 'traffic';
}

/** Where the user dropped boxes, by node id. */
export type MovedNodes = Record<string, { x: number; y: number }>;

/** Keeps a dragged box across the canvas; dragging one lower makes the canvas taller. */
export function clampToCanvas(x: number, y: number): { x: number; y: number } {
  return {
    x: Math.round(Math.min(Math.max(x, 0), CANVAS_WIDTH - NODE_WIDTH)),
    y: Math.round(Math.min(Math.max(y, TOP), MOST_HEIGHT - NODE_HEIGHT)),
  };
}

/** How tall the canvas is for these boxes: never shorter than the least. */
export function canvasHeight(nodes: readonly { y: number }[]): number {
  const lowest = Math.max(0, ...nodes.map((node) => node.y + NODE_HEIGHT + BOTTOM));
  return Math.max(CANVAS_HEIGHT, lowest);
}

/** What the map needs to know about the app itself, beyond its resources. */
export interface MapProject {
  source: string;
  sourceDetail: string;
  /** The logo for where the code comes from, such as `github`. */
  sourceIcon?: string;
  /** The repository's page, when the code is on GitHub. */
  sourceUrl?: string;
  /** The project's own repository, `owner/name`, when its code is on GitHub or a clone of it. */
  sourceRepo?: string;
  /** The public address, as a full URL. */
  urlHref?: string;
  url: string | null;
  urlDetail: string;
}

const RUNTIME_KINDS: ResourceKind[] = ['web', 'service', 'job'];

/** A domain is how visitors come in; code runs in the middle; the rest keeps data. */
function columnOf(kind: ResourceKind): 0 | 1 | 2 {
  if (kind === 'domain') return 0;
  return RUNTIME_KINDS.includes(kind) ? 1 : 2;
}

export function layoutMap(
  project: MapProject,
  map: ProjectMap,
  moved: MovedNodes = {},
): { nodes: MapNode[]; edges: MapEdge[] } {
  const runtimeResources = map.nodes.filter((resource) => columnOf(resource.kind) === 1);
  const dataResources = map.nodes.filter((resource) => columnOf(resource.kind) === 2);
  const domainResources = map.nodes.filter((resource) => columnOf(resource.kind) === 0);

  // A project can deploy from several repositories (a provider that builds one
  // on push, a monorepo's services): each other repository gets its own box,
  // and each service draws its line from where its code really is.
  const sourceOf = (resource: Resource): string =>
    resource.repo && resource.repo.toLowerCase() !== project.sourceRepo?.toLowerCase()
      ? `repo:${resource.repo}`
      : 'source';
  const otherRepos = [...new Set(runtimeResources.map(sourceOf).filter((id) => id !== 'source'))];
  const projectSourceUsed =
    runtimeResources.length === 0 ||
    runtimeResources.some((resource) => sourceOf(resource) === 'source');

  const projectSource: Omit<MapNode, 'x' | 'y'>[] = [
    {
      id: 'source',
      kind: 'source',
      title: project.source,
      detail: project.sourceDetail,
      status: project.sourceIcon ? 'live' : 'unknown',
      provider: null,
      ...(project.sourceIcon ? { icon: project.sourceIcon } : {}),
      ...(project.sourceUrl ? { href: project.sourceUrl } : {}),
    },
  ];
  const entryNodes = projectSourceUsed ? projectSource : [];
  for (const id of otherRepos) {
    const repo = id.slice('repo:'.length);
    entryNodes.push({
      id,
      kind: 'source',
      title: repo,
      detail: 'GitHub · its provider deploys each push',
      status: 'live',
      provider: null,
      icon: 'github',
      href: `https://github.com/${repo}`,
    });
  }
  if (project.url) {
    entryNodes.push({
      id: 'address',
      kind: 'address',
      title: project.url,
      detail: project.urlDetail,
      status: 'live',
      provider: runtimeResources[0]?.provider ?? null,
      ...(project.urlHref ? { href: project.urlHref } : {}),
    });
  }

  const nodes = [
    ...placeColumn(0, [...entryNodes, ...domainResources.map(resourceNode)]),
    ...placeColumn(1, runtimeResources.map(resourceNode)),
    ...placeColumn(2, dataResources.map(resourceNode)),
  ];
  // Once the person arranged the map, every box they placed stays put; one
  // added since joins its column below the lowest box there.
  const arranged = Object.keys(moved).length > 0;
  const lowest = new Map<number, number>();
  for (const node of nodes) {
    const at = moved[node.id];
    if (!at) continue;
    lowest.set(node.x, Math.max(lowest.get(node.x) ?? 0, at.y + NODE_HEIGHT));
  }
  for (const node of nodes) {
    const at = moved[node.id];
    if (at) {
      node.x = at.x;
      node.y = at.y;
    } else if (arranged) {
      const below = lowest.get(node.x);
      if (below !== undefined) {
        node.y = below + ROW - NODE_HEIGHT;
        lowest.set(node.x, node.y + NODE_HEIGHT);
      }
    }
  }
  const byId = new Map(nodes.map((node) => [node.id, node]));

  const edges: MapEdge[] = [];
  function connect(fromId: string, toId: string, label: string, style: MapEdge['style']): void {
    const from = byId.get(fromId);
    const to = byId.get(toId);
    if (from && to) edges.push(edgeBetween(from, to, label, style));
  }

  for (const resource of runtimeResources) {
    connect(sourceOf(resource), resource.id, 'deploys', 'deploys');
  }
  // The address leads to what serves it: the resource the provider reports at that
  // address; failing that, the web resources, or on providers that call everything a
  // service (Railway), the services. Scheduled jobs serve nothing.
  const at = runtimeResources.filter(
    (resource) => resource.url && project.url && hostOf(resource.url) === hostOf(project.url),
  );
  const web = runtimeResources.filter((candidate) => candidate.kind === 'web');
  const served =
    at.length > 0
      ? at
      : web.length > 0
        ? web
        : runtimeResources.filter((resource) => resource.kind === 'service');
  for (const resource of served) connect('address', resource.id, 'route', 'traffic');
  for (const link of map.edges) connect(link.from, link.to, link.via, 'traffic');

  return { nodes, edges };
}

function resourceNode(resource: Resource): Omit<MapNode, 'x' | 'y'> {
  return {
    id: resource.id,
    kind: 'resource',
    title: resource.name,
    detail: [resource.product, resource.usage].filter(Boolean).join(' · '),
    status: resource.status,
    provider: resource.provider,
  };
}

/** Spreads a column's nodes evenly down the canvas, which grows for a long column. */
function placeColumn(column: number, nodes: Omit<MapNode, 'x' | 'y'>[]): MapNode[] {
  const slot = Math.max((CANVAS_HEIGHT - TOP) / Math.max(nodes.length, 1), ROW);
  return nodes.map((node, index) => ({
    ...node,
    x: columnX(column),
    y: Math.round(TOP + slot * (index + 0.5) - NODE_HEIGHT / 2),
  }));
}

/**
 * An S-curve between two boxes, from the side of one that faces the other.
 * Normally right edge to left edge; flipped if a box was dragged past it.
 */
function edgeBetween(from: MapNode, to: MapNode, label: string, style: MapEdge['style']): MapEdge {
  const leftToRight = from.x <= to.x;
  const startX = leftToRight ? from.x + NODE_WIDTH : from.x;
  const startY = from.y + NODE_HEIGHT / 2;
  const endX = leftToRight ? to.x : to.x + NODE_WIDTH;
  const endY = to.y + NODE_HEIGHT / 2;
  const midX = (startX + endX) / 2;

  return {
    id: `${from.id}>${to.id}`,
    path: `M${startX} ${startY} C${midX} ${startY} ${midX} ${endY} ${endX} ${endY}`,
    label,
    labelX: midX,
    labelY: (startY + endY) / 2,
    style,
  };
}

/** `api.example.com` from `https://api.example.com/`, or the text as given. */
function hostOf(address: string): string {
  return address
    .replace(/^https?:\/\//, '')
    .replace(/\/.*$/, '')
    .toLowerCase();
}
