/**
 * Where the person arranged a project's map, kept on this computer so the map
 * looks the same next time. Once any box is moved, every box's place is kept,
 * so a resource added later joins below the others instead of shuffling the
 * arrangement. Reset puts every box back.
 */
import { useCallback, useState } from 'react';
import type { MovedNodes } from './layout';

export function useMovedNodes(projectId: string) {
  const key = `hoista.map.${projectId}`;
  const [state, setState] = useState<{ key: string; moved: MovedNodes }>(() => ({
    key,
    moved: load(key),
  }));
  let moved = state.moved;
  if (state.key !== key) {
    // Another project's map: read its arrangement from this render on.
    moved = load(key);
    setState({ key, moved });
  }

  /** Keeps every box's place, as the canvas reports them after a drop. */
  const arrange = useCallback(
    (positions: MovedNodes) => {
      setState({ key, moved: positions });
      store(key, positions);
    },
    [key],
  );

  const reset = useCallback(() => {
    setState({ key, moved: {} });
    store(key, {});
  }, [key]);

  return { moved, arrange, reset, hasMoved: Object.keys(moved).length > 0 };
}

function isPosition(value: unknown): value is { x: number; y: number } {
  if (typeof value !== 'object' || value === null) return false;
  const { x, y } = value as { x?: unknown; y?: unknown };
  return Number.isFinite(x) && Number.isFinite(y);
}

// Storage can be unavailable or full; the map then lays itself out again.
function load(key: string): MovedNodes {
  try {
    const saved = localStorage.getItem(key);
    const all: unknown = saved ? JSON.parse(saved) : {};
    if (typeof all !== 'object' || all === null) return {};
    const moved: MovedNodes = {};
    for (const [node, position] of Object.entries(all as Record<string, unknown>)) {
      if (isPosition(position)) moved[node] = { x: position.x, y: position.y };
    }
    return moved;
  } catch {
    return {};
  }
}

function store(key: string, moved: MovedNodes): void {
  try {
    if (Object.keys(moved).length === 0) localStorage.removeItem(key);
    else localStorage.setItem(key, JSON.stringify(moved));
  } catch {
    // Not remembered this time; the arrangement still holds until Hoista closes.
  }
}
