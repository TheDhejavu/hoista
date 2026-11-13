/**
 * A provider's answer, laid out to read: lists of things as tables (the provider's
 * paged lists unwrapped), single things as labelled facts, and plain values
 * as text. Nested detail stays one click away.
 */
import { createContext, useContext } from 'react';
import type { FieldWords } from '@shared/providers/types';
import { RowMenu, type RowMenuItem } from '@ui/components/RowMenu';
import { humanName } from './OperationDialog';
import styles from './Result.module.css';
import shared from './provider.module.css';

/** Rows a table of results shows; the rest are counted. */
const MAX_ROWS = 200;

/** The provider's own words for its fields, from its entry; provided by its tab. */
export const FieldWordsContext = createContext<FieldWords | undefined>(undefined);

function label(words: FieldWords | undefined, key: string): string {
  return words?.labels[key] ?? humanName(key);
}

/** A field's value as people read it: the provider's word for it, with its unit. */
function worded(words: FieldWords | undefined, key: string, value: string): string {
  // A time some providers give as seconds since 1970 (Modal's `created_at`).
  if (/(_at|At)$/.test(key) && /^\d{9,11}(\.\d+)?$/.test(value)) {
    return new Date(Number(value) * 1000).toLocaleString('en-US', {
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    });
  }
  // Providers' constants (`EMAIL`, `ON_FAILURE`) read as words unless the provider has its own.
  const said =
    words?.values[value] ??
    (/^[A-Z][A-Z0-9_]+$/.test(value) && value.length > 2
      ? `${value.charAt(0)}${value.slice(1).toLowerCase().replace(/_/g, ' ')}`
      : value);
  const unit = words?.units[key];
  return unit && said !== '—' ? `${said} ${unit}`.replace(/ s$/, 's') : said;
}

/** Ids are for machines; a table with other columns leaves them out. */
function isId(key: string): boolean {
  return key === 'id' || key.endsWith('Id');
}

type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

/** Unwraps Railway's `{ edges: [{ node }] }` lists. */
function rows(value: Json): Json[] | null {
  if (Array.isArray(value)) return value;
  if (value && typeof value === 'object' && Array.isArray(value.edges)) {
    return value.edges.map((edge) =>
      edge && typeof edge === 'object' && !Array.isArray(edge) && 'node' in edge
        ? (edge.node ?? null)
        : edge,
    );
  }
  return null;
}

function scalar(value: Json): value is null | boolean | number | string {
  return value === null || typeof value !== 'object';
}

function text(value: Json): string {
  if (value === null) return '—';
  if (typeof value === 'boolean') return value ? 'Yes' : 'No';
  if (typeof value === 'string' && /^\d{4}-\d\d-\d\dT/.test(value)) {
    return new Date(value).toLocaleString('en-US', {
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    });
  }
  if (typeof value === 'number' || typeof value === 'string') return String(value);
  return JSON.stringify(value);
}

/** Nested objects that are only `{ id }` read as that id. */
function cell(value: Json): string {
  if (scalar(value)) return text(value);
  if (!Array.isArray(value) && Object.keys(value).length === 1 && 'id' in value) {
    return text(value.id ?? null);
  }
  const list = rows(value);
  if (list) {
    if (list.length === 0) return 'None';
    // A short list of words or names reads as that list.
    const names = list.map((item) =>
      scalar(item)
        ? text(item)
        : !Array.isArray(item) && typeof item.name === 'string'
          ? item.name
          : null,
    );
    if (names.every((name): name is string => name !== null)) {
      return names.length > 3
        ? `${names.slice(0, 3).join(', ')} +${String(names.length - 3)}`
        : names.join(', ');
    }
    return `${String(list.length)} ${list.length === 1 ? 'item' : 'items'}`;
  }
  if (!Array.isArray(value) && typeof value.name === 'string') return value.name;
  return '…';
}

/** What a row of a table offers in its ⋮ menu. */
export interface RowAction {
  label: string;
  danger?: boolean;
  onSelect: (row: Record<string, unknown>) => void;
}

/** `empty` is what a nothing answer says: `Done.` after a change, by default. */
/**
 * What a provider's command-line tool printed, without the wrapper around it
 * (`{command, messages, output}`): its JSON read as data, so `fly status
 * --json` shows as fields and tables, not the command and a collapsed blob.
 */
function commandOutput(value: Json): Json {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return value;
  if (!('command' in value) || !('output' in value)) return value;
  const output = value.output ?? null;
  if (typeof output === 'string') {
    const trimmed = output.trim();
    if (trimmed.startsWith('{') || trimmed.startsWith('[')) {
      try {
        return JSON.parse(trimmed) as Json;
      } catch {
        return trimmed;
      }
    }
    if (trimmed) return trimmed;
  } else if (output !== null && !(Array.isArray(output) && output.length === 0)) {
    return output;
  }
  // Nothing printed: what the tool said instead, if anything.
  const messages = value.messages;
  const said = Array.isArray(messages)
    ? messages.filter((line): line is string => typeof line === 'string')
    : [];
  return said.length > 0 ? said.join('\n') : null;
}

export function Result({
  value,
  empty = 'Done.',
  rowActions,
}: {
  value: unknown;
  empty?: string;
  rowActions?: RowAction[];
}) {
  const json = commandOutput((value ?? null) as Json);
  if (scalar(json)) {
    return <p className={styles.plain}>{json === null ? empty : text(json)}</p>;
  }
  const list = rows(json);
  if (list)
    return list.length === 0 && empty !== 'Done.' ? (
      <p className={styles.plain}>{empty}</p>
    ) : (
      <Table items={list} rowActions={rowActions} />
    );
  return <ObjectView value={json as { [key: string]: Json }} />;
}

function Table({ items, rowActions }: { items: Json[]; rowActions?: RowAction[] | undefined }) {
  const words = useContext(FieldWordsContext);
  if (items.length === 0) return <p className={styles.plain}>None.</p>;
  if (items.every(scalar)) {
    return <p className={styles.plain}>{items.map(text).join(', ')}</p>;
  }
  const objects = items.filter(
    (item): item is { [key: string]: Json } =>
      item !== null && typeof item === 'object' && !Array.isArray(item),
  );
  // Columns: the fields most items have, simple ones first, at most eight.
  const counts = new Map<string, number>();
  for (const item of objects) {
    for (const [key, field] of Object.entries(item)) {
      if (field !== null) counts.set(key, (counts.get(key) ?? 0) + (scalar(field) ? 2 : 1));
    }
  }
  const preferred = ['name', 'status', 'createdAt', 'id'];
  const named = [...counts.keys()].some((key) => !isId(key));
  const columns = [...counts.entries()]
    .filter(([key]) => !named || !isId(key))
    .sort(
      ([a, x], [b, y]) =>
        (preferred.includes(b) ? 100 - preferred.indexOf(b) : 0) -
          (preferred.includes(a) ? 100 - preferred.indexOf(a) : 0) || y - x,
    )
    // A column with nothing readable in any row is left out.
    .filter(([key]) => objects.some((item) => cell(item[key] ?? null) !== '…'))
    .slice(0, 8)
    .map(([key]) => key);
  return (
    <div className={styles.tableWrap}>
      <table className={styles.table}>
        <thead>
          <tr>
            {columns.map((column) => (
              <th key={column}>{label(words, column)}</th>
            ))}
            {rowActions && rowActions.length > 0 && (
              <th className={styles.menuCol}>
                <span className="visually-hidden">Actions</span>
              </th>
            )}
          </tr>
        </thead>
        <tbody>
          {objects.slice(0, MAX_ROWS).map((item, index) => (
            <tr key={typeof item.id === 'string' ? item.id : index}>
              {columns.map((column) => (
                <td key={column} className={isId(column) ? 'mono' : undefined}>
                  {worded(words, column, cell(item[column] ?? null))}
                </td>
              ))}
              {rowActions && rowActions.length > 0 && (
                <td className={styles.menuCol}>
                  <RowMenu
                    label={`More for ${typeof item.name === 'string' ? item.name : 'this row'}`}
                    items={rowActions.map((action): RowMenuItem => ({
                      label: action.label,
                      danger: action.danger ?? false,
                      onSelect: () => {
                        action.onSelect(item);
                      },
                    }))}
                  />
                </td>
              )}
            </tr>
          ))}
        </tbody>
      </table>
      {objects.length > MAX_ROWS && (
        <p className={shared.muted}>
          Showing {MAX_ROWS} of {objects.length}.
        </p>
      )}
    </div>
  );
}

function ObjectView({ value }: { value: { [key: string]: Json } }) {
  const words = useContext(FieldWordsContext);
  const entries = entriesOf(value);
  const simple = entries.filter(([, field]) => scalar(field) || cell(field) !== '…');
  const nested = entries.filter(([, field]) => !scalar(field) && cell(field) === '…');
  return (
    <div className={styles.object}>
      <dl className={styles.facts}>
        {simple.map(([key, field]) => (
          <div key={key} className={styles.fact}>
            <dt>{label(words, key)}</dt>
            <dd>{worded(words, key, cell(field))}</dd>
          </div>
        ))}
      </dl>
      {nested.map(([key, field]) => (
        <details key={key} className={styles.nested}>
          <summary>{label(words, key)}</summary>
          <Result value={field} />
        </details>
      ))}
      {entries.length === 0 && <p className={styles.plain}>Nothing to show.</p>}
    </div>
  );
}

function entriesOf(value: { [key: string]: Json }): [string, Json][] {
  return globalThis.Object.entries(value).filter(([key]) => key !== '__typename');
}
