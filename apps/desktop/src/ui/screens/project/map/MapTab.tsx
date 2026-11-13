/**
 * Map: every part of the app and how they connect, from where the code
 * comes from to where the data is kept. Click a resource to see its details.
 */
import { Minus, Plus } from 'lucide-react';
import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent,
  type RefObject,
} from 'react';
import { useStoredChoice } from '@ui/lib/storedChoice';
import type { ProjectDetail, ProjectMap, Resource } from '@shared/api';
import { runsCode } from '../resourceKinds';
import { useProject, useProjectMap } from '@ui/api/projects';
import { QueryState } from '@ui/components/QueryState';
import { Skeleton } from '@ui/components/Skeleton';
import { PillSelect, type Option } from '@ui/components/Select';
import { formatCount, hostOf, timeAgo } from '@ui/lib/format';
import { useRequiredParam } from '@ui/lib/useRequiredParam';
import { Button } from '@ui/components/Button';
import { openExternal } from '@ui/api/system';
import { cx } from '@ui/lib/cx';
import { MapCanvas } from './MapCanvas';
import { SelectedResource } from './SelectedResource';
import { EntryPanel } from './EntryPanel';
import { repoUrl, splitSource } from '@ui/lib/links';
import { CANVAS_HEIGHT, CANVAS_WIDTH, type MapProject } from './layout';
import { useMovedNodes } from './useMovedNodes';
import styles from './MapTab.module.css';

const SOURCE_LABEL = {
  folder: 'Local folder',
  github: 'GitHub',
  unlinked: 'no linked source',
} as const;

type MapFilter = 'all' | 'problems' | 'data';

const FILTER_CHOICES: Option<MapFilter>[] = [
  { value: 'all', label: 'Everything' },
  { value: 'problems', label: 'Problems' },
  { value: 'data', label: 'Databases and storage' },
];

const NONE_MATCH: Record<MapFilter, string> = {
  all: '',
  problems: 'Nothing has a problem right now.',
  data: 'This project keeps no data at its providers.',
};

export function MapTab() {
  const projectId = useRequiredParam('projectId');
  const project = useProject(projectId);
  const map = useProjectMap(projectId);
  const [filter, setFilter] = useState<MapFilter>('all');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const layout = useMovedNodes(projectId);

  const firstData =
    map.data?.nodes.find((resource) => resource.kind === 'database') ?? map.data?.nodes[0];
  const shownId = selectedId ?? firstData?.id ?? null;

  const board = useRef<HTMLDivElement>(null);
  const fit = useFitZoom(board);
  const [zoom, setZoom] = useState<number | null>(null);
  const shownZoom = zoom ?? fit;
  const size = useBoardHeight(board);

  return (
    <div className={styles.layout}>
      <div
        ref={board}
        className={styles.board}
        style={
          size.height === null
            ? { minHeight: CANVAS_HEIGHT * shownZoom + 150 }
            : { height: size.height }
        }
      >
        <div className={styles.overlay}>
          <PillSelect label="Show" value={filter} options={FILTER_CHOICES} onChange={setFilter} />
          {filter !== 'all' &&
            map.data &&
            !map.data.nodes.some((resource) => matchesFilter(resource, filter)) && (
              <span className={styles.filterNote}>{NONE_MATCH[filter]}</span>
            )}
          <span className={styles.spacer} />
          {layout.hasMoved && (
            <Button variant="ghost" onClick={layout.reset}>
              Reset layout
            </Button>
          )}
          <button
            type="button"
            className={styles.zoomButton}
            aria-label="Zoom out"
            onClick={() => {
              setZoom(Math.max(MIN_ZOOM, shownZoom - ZOOM_STEP));
            }}
          >
            <Minus size={13} aria-hidden="true" />
          </button>
          <button
            type="button"
            className={styles.zoomButton}
            aria-label="Zoom in"
            onClick={() => {
              setZoom(Math.min(MAX_ZOOM, shownZoom + ZOOM_STEP));
            }}
          >
            <Plus size={13} aria-hidden="true" />
          </button>
          <button
            type="button"
            className={styles.fitButton}
            onClick={() => {
              setZoom(null);
            }}
          >
            Fit
          </button>
        </div>

        <div className={styles.canvasArea}>
          <QueryState
            query={map}
            skeleton={
              <div className={styles.mapSkeleton} aria-hidden="true">
                <Skeleton width={170} height={64} />
                <Skeleton width={170} height={64} />
                <Skeleton width={170} height={64} />
              </div>
            }
          >
            {(data) =>
              project.data && (
                <MapCanvas
                  project={mapProject(project.data)}
                  map={data}
                  isShown={(resource) => matchesFilter(resource, filter)}
                  selectedId={shownId}
                  onSelect={(id) => {
                    // Another repository's box opens it on GitHub.
                    if (id.startsWith('repo:')) {
                      void openExternal(`https://github.com/${id.slice('repo:'.length)}`);
                      return;
                    }
                    setSelectedId(id);
                  }}
                  moved={layout.moved}
                  onArrange={layout.arrange}
                  zoom={shownZoom}
                />
              )
            }
          </QueryState>
        </div>
        <div className={styles.caption}>
          {map.data && Object.keys(map.data.traffic).length > 0
            ? 'Moving dots show live traffic. Drag boxes to arrange them, and the bottom edge for more room.'
            : 'Lines show how resources connect, read from each provider. Drag boxes to arrange them, and the bottom edge for more room.'}
        </div>
        <div
          className={cx(styles.resize, size.resizing && styles.resizing)}
          role="separator"
          aria-orientation="horizontal"
          aria-label="Map height"
          aria-valuemin={MIN_BOARD}
          aria-valuemax={MAX_BOARD}
          aria-valuenow={size.height ?? undefined}
          tabIndex={0}
          title="Drag to make the map taller or shorter. Double-click to reset."
          onPointerDown={size.start}
          onKeyDown={size.key}
          onDoubleClick={size.reset}
        />
      </div>

      {(shownId === 'source' || shownId === 'address') && project.data ? (
        <EntryPanel which={shownId} detail={project.data} />
      ) : (
        shownId && (
          <SelectedResource
            projectId={projectId}
            resourceId={shownId}
            unlinked={unlinkedFor(shownId, map.data)}
            onRemoved={() => {
              setSelectedId(null);
            }}
          />
        )
      )}
    </div>
  );
}

/**
 * The project's services to choose from when nothing on the map connects to a
 * resource that doesn't run code; undefined when it's connected or runs code.
 */
function unlinkedFor(
  id: string,
  map: ProjectMap | undefined,
): { id: string; name: string }[] | undefined {
  const resource = map?.nodes.find((node) => node.id === id);
  if (!map || !resource || runsCode(resource.kind)) return undefined;
  if (map.edges.some((edge) => edge.from === id || edge.to === id)) return undefined;
  return map.nodes
    .filter((node) => runsCode(node.kind))
    .map((node) => ({ id: node.id, name: node.name }));
}

const MIN_ZOOM = 0.5;
const MAX_ZOOM = 1.5;
const ZOOM_STEP = 0.1;
/** Room left and right of the map inside the board. */
const BOARD_PADDING = 48;

/** The zoom that fits the whole map across the board, never above full size. */
const MIN_BOARD = 320;
const MAX_BOARD = 2400;

function isHeight(value: unknown): value is number | null {
  return value === null || (typeof value === 'number' && value >= MIN_BOARD && value <= MAX_BOARD);
}

/**
 * The map's height when the person dragged its bottom edge, remembered on this
 * computer; null keeps the height that fits the map.
 */
function useBoardHeight(board: RefObject<HTMLDivElement | null>) {
  const [saved, save] = useStoredChoice<number | null>('map.height', null, isHeight);
  const [live, setLive] = useState<number | null>(null);
  const clamp = (value: number): number =>
    Math.round(Math.min(MAX_BOARD, Math.max(MIN_BOARD, value)));
  const current = (): number => saved ?? board.current?.offsetHeight ?? MIN_BOARD;

  function start(event: PointerEvent<HTMLDivElement>): void {
    if (event.button !== 0) return;
    event.preventDefault();
    const target = event.currentTarget;
    target.setPointerCapture(event.pointerId);
    const startY = event.clientY;
    const startHeight = current();
    let next = startHeight;
    setLive(startHeight);
    const move = (moveEvent: globalThis.PointerEvent): void => {
      next = clamp(startHeight + moveEvent.clientY - startY);
      setLive(next);
    };
    const end = (): void => {
      target.removeEventListener('pointermove', move);
      target.removeEventListener('pointerup', end);
      target.removeEventListener('pointercancel', end);
      setLive(null);
      save(next);
    };
    target.addEventListener('pointermove', move);
    target.addEventListener('pointerup', end);
    target.addEventListener('pointercancel', end);
  }

  function key(event: KeyboardEvent<HTMLDivElement>): void {
    const step = event.key === 'ArrowDown' ? 40 : event.key === 'ArrowUp' ? -40 : 0;
    if (step === 0) return;
    event.preventDefault();
    save(clamp(current() + step));
  }

  return {
    height: live ?? saved,
    resizing: live !== null,
    start,
    key,
    reset: () => {
      save(null);
    },
  };
}

function useFitZoom(board: RefObject<HTMLDivElement | null>): number {
  const [fit, setFit] = useState(1);
  useEffect(() => {
    const element = board.current;
    if (!element) return;
    const observer = new ResizeObserver(([entry]) => {
      if (!entry) return;
      const width = entry.contentRect.width - BOARD_PADDING;
      setFit(Math.max(MIN_ZOOM, Math.min(1, width / CANVAS_WIDTH)));
    });
    observer.observe(element);
    return () => {
      observer.disconnect();
    };
  }, [board]);
  return fit;
}

function matchesFilter(resource: Resource, filter: MapFilter): boolean {
  switch (filter) {
    case 'all':
      return true;
    case 'problems':
      return (
        resource.status === 'failing' ||
        resource.status === 'paused' ||
        resource.status === 'unknown'
      );
    case 'data':
      return resource.kind === 'database' || resource.kind === 'bucket';
  }
}

/** The project-level boxes on the left of the map: where code comes from and the public address. */
function mapProject(detail: ProjectDetail): MapProject {
  const { project } = detail;
  const lastDeploy = project.last_deploy
    ? `deployed ${timeAgo(project.last_deploy.at)}`
    : 'not deployed yet';
  const requests = detail.stats.requests;

  if (project.source.kind === 'unlinked' || !project.source.ref) {
    return {
      source: 'Code not linked',
      sourceDetail: 'Link a folder or repo in Settings',
      url: project.url ? hostOf(project.url) : null,
      ...(project.url ? { urlHref: project.url } : {}),
      urlDetail: requests === null ? 'Domain' : `${formatCount(requests)} requests · 7d`,
    };
  }
  if (project.source.kind === 'github') {
    return {
      source: splitSource(project.source.ref).repo,
      sourceRepo: splitSource(project.source.ref).repo,
      sourceDetail: `GitHub · ${lastDeploy}`,
      sourceIcon: 'github',
      ...(repoUrl(splitSource(project.source.ref).repo)
        ? { sourceUrl: repoUrl(splitSource(project.source.ref).repo) ?? '' }
        : {}),
      url: project.url ? hostOf(project.url) : null,
      ...(project.url ? { urlHref: project.url } : {}),
      urlDetail: requests === null ? 'Domain' : `${formatCount(requests)} requests · 7d`,
    };
  }
  // A folder that is a clone of a GitHub repository shows the repository,
  // the branch and commit checked out, and that it's built from this computer.
  const git = project.source.git;
  if (git) {
    const at = [git.branch, git.commit?.slice(0, 7)].filter(Boolean).join(' @ ');
    return {
      source: git.repository,
      sourceRepo: git.repository,
      sourceDetail: ['Local clone', at, git.temporary ? 'temporary folder' : lastDeploy]
        .filter(Boolean)
        .join(' · '),
      sourceIcon: 'github',
      ...(repoUrl(git.repository) ? { sourceUrl: repoUrl(git.repository) ?? '' } : {}),
      url: project.url ? hostOf(project.url) : null,
      ...(project.url ? { urlHref: project.url } : {}),
      urlDetail: requests === null ? 'Domain' : `${formatCount(requests)} requests · 7d`,
    };
  }
  return {
    source: project.source.ref,
    sourceDetail: `${SOURCE_LABEL[project.source.kind]} · ${lastDeploy}`,
    url: project.url ? hostOf(project.url) : null,
    ...(project.url ? { urlHref: project.url } : {}),
    urlDetail: requests === null ? 'Domain' : `${formatCount(requests)} requests · 7d`,
  };
}
