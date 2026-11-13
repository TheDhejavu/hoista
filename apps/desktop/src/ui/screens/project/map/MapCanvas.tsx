/**
 * Draws the laid-out map: column titles, edges with labels, and the boxes.
 * Boxes can be dragged anywhere on the canvas; their edges follow. A press
 * that doesn't move is a click and opens the resource.
 */
import { useRef, useState, type PointerEvent } from 'react';
import type { ProjectMap, Resource } from '@shared/api';
import { Dot, type Tone } from '@ui/components/Dot';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { cx } from '@ui/lib/cx';
import {
  CANVAS_WIDTH,
  COLUMN_TITLES,
  NODE_HEIGHT,
  NODE_WIDTH,
  clampToCanvas,
  canvasHeight,
  columnX,
  layoutMap,
  type MapProject,
  type MovedNodes,
  type MapEdge,
  type MapNode,
} from './layout';
import styles from './MapCanvas.module.css';

export function MapCanvas({
  project,
  map,
  isShown,
  selectedId,
  onSelect,
  moved,
  onArrange,
  zoom = 1,
}: {
  project: MapProject;
  map: ProjectMap;
  isShown: (resource: Resource) => boolean;
  selectedId: string | null;
  onSelect: (resourceId: string) => void;
  /** Where boxes were dropped before. */
  moved: MovedNodes;
  /** Called once when a box is dropped, with every box's place: the whole arrangement is kept. */
  onArrange: (positions: MovedNodes) => void;
  /** How much the map is scaled; drags are divided by it so boxes stay under the pointer. */
  zoom?: number;
}) {
  const { nodes: placed } = layoutMap(project, map, moved);
  const drag = useNodeDrag((nodeId, position) => {
    onArrange({
      ...Object.fromEntries(placed.map((node) => [node.id, { x: node.x, y: node.y }])),
      [nodeId]: position,
    });
  }, zoom);
  const { nodes, edges } = drag.live
    ? layoutMap(project, map, { ...moved, ...drag.live })
    : layoutMap(project, map, moved);
  const height = canvasHeight(nodes);
  const hidden = new Set(
    map.nodes.filter((resource) => !isShown(resource)).map((resource) => resource.id),
  );
  // While a filter is on, the boxes that aren't resources (where the code
  // comes from, the public address) fade with everything that doesn't match.
  const filtering = hidden.size > 0;

  return (
    // The outer box takes the scaled size, so the map centres and scrolls correctly.
    <div className={styles.frame} style={{ width: CANVAS_WIDTH * zoom, height: height * zoom }}>
      <div
        className={styles.canvas}
        style={{ width: CANVAS_WIDTH, height, transform: `scale(${zoom})` }}
      >
        {COLUMN_TITLES.map((title, column) => (
          <span key={title} className={styles.columnTitle} style={{ left: columnX(column) }}>
            {title}
          </span>
        ))}

        <svg className={styles.lines} width={CANVAS_WIDTH} height={height} aria-hidden="true">
          {edges.map((edge) => (
            <EdgeLine key={edge.id} edge={edge} />
          ))}
        </svg>
        {edges.map((edge) => (
          <span
            key={edge.id}
            className={styles.edgeLabel}
            style={{ left: edge.labelX, top: edge.labelY }}
          >
            {edge.label}
          </span>
        ))}

        {nodes.map((node) => (
          <NodeBox
            key={node.id}
            node={node}
            isSelected={node.id === selectedId}
            isDimmed={
              hidden.has(node.id) ||
              (filtering && !map.nodes.some((resource) => resource.id === node.id))
            }
            isDragging={drag.draggingId === node.id}
            onSelect={(id) => {
              if (!drag.justDragged()) onSelect(id);
            }}
            onPointerDown={(event) => {
              drag.start(event, node);
            }}
          />
        ))}
      </div>
    </div>
  );
}

/** Movement under this many pixels counts as a click, not a drag. */
const DRAG_THRESHOLD = 4;

/**
 * Follows the pointer while a box is held. The box's live position is kept
 * here and only reported to `onMove` when it is dropped.
 */
function useNodeDrag(
  onMove: (nodeId: string, position: { x: number; y: number }) => void,
  zoom: number,
) {
  const [live, setLive] = useState<MovedNodes | null>(null);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const draggedRecently = useRef(false);

  function start(event: PointerEvent<HTMLElement>, node: MapNode): void {
    if (event.button !== 0) return;
    const target = event.currentTarget;
    const startX = event.clientX;
    const startY = event.clientY;
    let position = { x: node.x, y: node.y };
    let isDragging = false;
    draggedRecently.current = false;
    target.setPointerCapture(event.pointerId);

    const handleMove = (moveEvent: globalThis.PointerEvent): void => {
      const dx = (moveEvent.clientX - startX) / zoom;
      const dy = (moveEvent.clientY - startY) / zoom;
      if (!isDragging && Math.hypot(dx, dy) < DRAG_THRESHOLD) return;
      isDragging = true;
      position = clampToCanvas(node.x + dx, node.y + dy);
      setDraggingId(node.id);
      setLive({ [node.id]: position });
    };
    const handleUp = (): void => {
      target.removeEventListener('pointermove', handleMove);
      target.removeEventListener('pointerup', handleUp);
      target.removeEventListener('pointercancel', handleUp);
      if (isDragging) {
        draggedRecently.current = true;
        onMove(node.id, position);
      }
      setDraggingId(null);
      setLive(null);
    };
    target.addEventListener('pointermove', handleMove);
    target.addEventListener('pointerup', handleUp);
    target.addEventListener('pointercancel', handleUp);
  }

  /** True once, right after a drag, so the click that ends it doesn't open the resource. */
  function justDragged(): boolean {
    const was = draggedRecently.current;
    draggedRecently.current = false;
    return was;
  }

  return { live, draggingId, start, justDragged };
}

function EdgeLine({ edge }: { edge: MapEdge }) {
  if (edge.style === 'deploys') return <path d={edge.path} className={styles.edgeDeploys} />;

  return (
    <>
      <path d={edge.path} className={styles.edgeTraffic} />
      <path d={edge.path} className={styles.edgeFlow} />
    </>
  );
}

const STATUS_TONES: Record<MapNode['status'], Tone> = {
  live: 'good',
  sleeping: 'neutral',
  idle: 'neutral',
  failing: 'bad',
  paused: 'warn',
  deploying: 'warn',
  not_deployed: 'neutral',
  stopped: 'neutral',
  unknown: 'neutral',
};

function NodeBox({
  node,
  isSelected,
  isDimmed,
  isDragging,
  onSelect,
  onPointerDown,
}: {
  node: MapNode;
  isSelected: boolean;
  isDimmed: boolean;
  isDragging: boolean;
  onSelect: (resourceId: string) => void;
  onPointerDown: (event: PointerEvent<HTMLElement>) => void;
}) {
  const className = cx(
    styles.node,
    isSelected && styles.selected,
    isDimmed && styles.dimmed,
    isDragging && styles.dragging,
  );
  const position = { left: node.x, top: node.y, width: NODE_WIDTH, height: NODE_HEIGHT };
  const content = (
    <>
      {node.icon || node.provider ? (
        <ServiceIcon id={node.icon ?? node.provider ?? ''} />
      ) : (
        <FolderIcon />
      )}
      <span className={styles.nodeText}>
        <span className={styles.nodeTitle}>{node.title}</span>
        <span className={styles.nodeDetail}>{node.detail}</span>
      </span>
      <Dot tone={STATUS_TONES[node.status]} />
    </>
  );

  return (
    <button
      type="button"
      className={className}
      style={position}
      aria-pressed={isSelected}
      onPointerDown={onPointerDown}
      onClick={() => {
        onSelect(node.id);
      }}
    >
      {content}
    </button>
  );
}

function FolderIcon() {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="var(--text-2)"
      strokeWidth="1.8"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M3 6.5A1.5 1.5 0 0 1 4.5 5h4.2l2 2.2h8.8A1.5 1.5 0 0 1 21 8.7v9.8a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5z" />
    </svg>
  );
}
