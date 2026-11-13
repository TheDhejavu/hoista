import { useState } from 'react';
import type { ProviderOperation, ProviderTarget } from '@shared/api';
import { useProviderRead } from '@ui/api/providerOperations';
import { SkeletonLines } from '@ui/components/Skeleton';
import { Result, type RowAction } from './Result';
import { type View } from './sections';
import styles from './provider.module.css';
import { Refusal, type RunSpec, Runner } from './Runner';

/** What the provider reports for one thing, read as soon as the section opens. */
export function ViewCard({
  view,
  target,
  byName,
  args,
}: {
  view: View;
  target: ProviderTarget;
  byName: Map<string, ProviderOperation>;
  args: Record<string, unknown>;
}) {
  const read = useProviderRead(target, view.op, args);
  const [running, setRunning] = useState<RunSpec | null>(null);
  const value =
    view.field && read.data && typeof read.data === 'object'
      ? (read.data as Record<string, unknown>)[view.field]
      : read.data;
  // Each row's own actions, with the row filled in.
  const rowActions: RowAction[] = (view.rowActions ?? []).flatMap((action) => {
    const op = byName.get(action.op);
    if (!op) return [];
    return [
      {
        label: action.label,
        danger: op.risk === 'destroy',
        onSelect: (row: Record<string, unknown>) => {
          const from = row[action.from ?? 'id'];
          setRunning({
            op,
            label: `${action.label}${typeof row.name === 'string' ? ` ${row.name}` : ''}`,
            preset: { [action.arg ?? 'id']: from },
            confirmName: typeof row.name === 'string' ? row.name : 'delete',
          });
        },
      },
    ];
  });
  return (
    <section className={styles.card}>
      <div className={styles.cardHead}>
        <h4>{view.title}</h4>
        {view.about && <span className={styles.cardAbout}>{view.about}</span>}
      </div>
      <div className={styles.cardBody}>
        {read.isPending && <SkeletonLines />}
        {read.error && <Refusal error={read.error} />}
        {value !== undefined && (
          <Result
            value={value}
            empty={view.empty ?? 'Nothing reported.'}
            rowActions={rowActions.length > 0 ? rowActions : undefined}
          />
        )}
      </div>
      {running && (
        <Runner
          spec={running}
          target={target}
          onClose={() => {
            setRunning(null);
          }}
        />
      )}
    </section>
  );
}
