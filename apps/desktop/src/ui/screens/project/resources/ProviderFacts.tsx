/**
 * A resource's own facts at its provider, read through the provider's operations as
 * its entry in `shared/providers` lists them: a project's version and region, its
 * branches, a domain's DNS records, a Worker's deployments. A section the
 * provider doesn't answer is left out, never shown empty.
 */
import type { ReactNode } from 'react';
import type { ProviderTarget, Resource } from '@shared/api';
import { providerEntry } from '@shared/providers';
import type { FactFormat, ProviderFact, ProviderFacts } from '@shared/providers/types';
import { useProviderRead } from '@ui/api/providerOperations';
import { formatUnit } from '../metrics/ChartMetrics';
import { timeAgo } from '@ui/lib/format';
import { providerTime } from '@shared/time';
import { Rows, Section } from './ResourceSection';
import styles from './ResourceScreen.module.css';

/** The sections a resource's provider describes for it. */
export function factsFor(resource: Pick<Resource, 'provider' | 'kind' | 'provider_ref'>) {
  return (providerEntry(resource.provider)?.facts ?? []).filter(
    (facts) =>
      (!facts.on || facts.on.includes(resource.kind)) &&
      (!facts.refs || facts.refs.some((ref) => resource.provider_ref.startsWith(ref))),
  );
}

/** The value at a dot path, or undefined. */
export function at(value: unknown, path: string): unknown {
  if (path === '') return value;
  let here: unknown = value;
  for (const key of path.split('.')) {
    if (here === null || typeof here !== 'object') return undefined;
    here = (here as Record<string, unknown>)[key];
  }
  return here;
}

/** A provider's time, as RFC 3339 text or Unix seconds or milliseconds. */
function time(value: unknown): string | null {
  if (typeof value === 'number') return new Date(providerTime(value)).toISOString();
  if (typeof value === 'string' && !Number.isNaN(Date.parse(value))) return value;
  return null;
}

/** A fact as text, or null when the provider didn't give it. */
export function written(value: unknown, as: FactFormat = 'text'): string | null {
  if (value === undefined || value === null || value === '') return null;
  const plain =
    typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean'
      ? String(value)
      : null;
  const number = typeof value === 'number' ? value : Number(value);
  switch (as) {
    case 'bool':
      return value === true || value === 'enabled' ? 'Yes' : value === false ? 'No' : plain;
    case 'bytes':
      return Number.isFinite(number) ? formatUnit('bytes', number) : null;
    case 'gb':
      return Number.isFinite(number) ? formatUnit('bytes', number * 1e9) : null;
    case 'mb':
      return Number.isFinite(number) ? formatUnit('bytes', number * 1e6) : null;
    case 'seconds':
      return Number.isFinite(number) ? formatUnit('seconds', number) : null;
    case 'cu':
      return Number.isFinite(number) ? `${String(number)} CU` : null;
    case 'ago': {
      const when = time(value);
      return when ? timeAgo(when) : null;
    }
    case 'join':
      return Array.isArray(value)
        ? value.filter((part) => typeof part === 'string').join(', ') || null
        : plain;
    case 'code':
    case 'text':
      return plain;
  }
}

function cell(fact: ProviderFact, value: unknown): ReactNode {
  const text = written(at(value, fact.path), fact.as);
  if (text === null) return <span className={styles.muted}>—</span>;
  return fact.as === 'code' ? <span className="mono">{text}</span> : text;
}

/** One section, read once and kept for a minute. */
export function FactsSection({ facts, target }: { facts: ProviderFacts; target: ProviderTarget }) {
  const read = useProviderRead(target, facts.op, facts.args ?? {});
  if (!read.data) return null;
  if (facts.rows) {
    const rows = facts.rows.flatMap((fact): [string, ReactNode][] =>
      written(at(read.data, fact.path), fact.as) === null
        ? []
        : [[fact.label, cell(fact, read.data)]],
    );
    if (rows.length === 0) return null;
    return (
      <Section title={facts.title}>
        <Rows rows={rows} />
      </Section>
    );
  }
  const list = at(read.data, facts.list ?? '');
  const items = Array.isArray(list) ? list.slice(0, facts.limit ?? 20) : [];
  if (items.length === 0 || !facts.columns) return null;
  const columns = facts.columns;
  return (
    <Section
      title={facts.title}
      action={
        Array.isArray(list) && list.length > items.length ? (
          <span className={styles.more}>
            {items.length} of {list.length}
          </span>
        ) : undefined
      }
    >
      <table className={styles.factTable}>
        <thead>
          <tr>
            {columns.map((column) => (
              <th key={column.label} scope="col">
                {column.label}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {items.map((item, index) => (
            <tr key={index}>
              {columns.map((column) => (
                <td key={column.label}>{cell(column, item)}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </Section>
  );
}
