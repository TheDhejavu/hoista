import { useContext, useMemo, useState } from 'react';
import type { ProviderOperation, ProviderTarget, ResourceKind } from '@shared/api';
import { useProviderRead } from '@ui/api/providerOperations';
import { Button } from '@ui/components/Button';
import { ProviderError } from './ProviderError';
import { SkeletonLines } from '@ui/components/Skeleton';
import { PillSelect } from '@ui/components/Select';
import { cx } from '@ui/lib/cx';
import { OperationDialog, humanName, isCommand, needsInput } from './OperationDialog';
import { Result } from './Result';
import styles from './OperationsExplorer.module.css';
import shared from './provider.module.css';
import { ResourceNameContext } from './Runner';
import { RISK_LABEL } from './ProviderTab';

/** Areas that fit each kind of resource, in the words providers use. */
const AREAS_FOR: Record<ResourceKind, string[]> = {
  web: ['Workers', 'Deployments', 'Projects', 'Projects', 'Machines', 'Pages'],
  service: ['Services', 'Deployments', 'Projects', 'Machines'],
  job: ['Workers', 'Cron', 'Deployments'],
  database: ['Database', 'D1', 'Managed Postgres', 'Projects'],
  bucket: ['R2', 'Storage', 'Buckets'],
  email: ['Email Routing', 'Domains'],
  domain: ['DNS', 'Domains'],
};

/** The area the list opens on: one that fits the resource, else the API's first. */
function startingArea(areas: string[], kind: ResourceKind): string | null {
  for (const wanted of AREAS_FOR[kind]) {
    const found = areas.find((name) => name === wanted);
    if (found) return found;
  }
  return areas[0] ?? null;
}

/** Every raw operation, by area, with search: for anything the sections don't cover. */
export function OperationsExplorer({
  ops,
  target,
  provider,
  kind,
  query,
  setQuery,
}: {
  query: string;
  setQuery: (query: string) => void;
  ops: ProviderOperation[];
  target: ProviderTarget;
  provider: string;
  kind: ResourceKind;
}) {
  const [area, setArea] = useState<string | null>(null);
  // The provider's API first; its command-line tool after, for what the API can't do.
  const areas = useMemo(() => {
    const counts = new Map<string, { count: number; command: boolean }>();
    for (const op of ops) {
      const seen = counts.get(op.area) ?? { count: 0, command: true };
      counts.set(op.area, { count: seen.count + 1, command: seen.command && isCommand(op.name) });
    }
    return [...counts.entries()]
      .sort(([a, x], [b, y]) => Number(x.command) - Number(y.command) || a.localeCompare(b))
      .map(([name, { count }]) => [name, count] as const);
  }, [ops]);
  const shownArea =
    area ??
    startingArea(
      areas.map(([name]) => name),
      kind,
    );
  const needle = query.trim().toLowerCase();
  const shown = ops.filter((op) =>
    needle
      ? `${op.name} ${humanName(op.name)} ${op.description}`.toLowerCase().includes(needle)
      : op.area === shownArea,
  );
  const order = { read: 0, change: 1, destroy: 2 };
  shown.sort((a, b) => order[a.risk] - order[b.risk] || a.name.localeCompare(b.name));
  return (
    <div className={shared.section}>
      <div className={shared.sectionHead}>
        <div>
          <h3>Every operation</h3>
          <p className={shared.lede}>
            {ops.every((op) => isCommand(op.name))
              ? `Everything ${provider}’s command-line tool can do here.`
              : ops.some((op) => isCommand(op.name))
                ? `${provider}’s whole API, and its command-line tool for what the API can’t do.`
                : `${provider}’s whole API, for anything the sections don’t cover.`}{' '}
            Hoista fills in which resource it runs on, and removes secrets from answers.
          </p>
        </div>
        <div className={shared.actions}>
          <PillSelect
            label="Area"
            value={needle ? null : shownArea}
            options={[
              ...(needle ? [{ value: null as string | null, label: 'Search results' }] : []),
              ...areas.map(([name, count]) => ({
                value: name,
                label: `${name} · ${String(count)}`,
              })),
            ]}
            onChange={(next) => {
              setQuery('');
              setArea(next);
            }}
          />
          <input
            className={styles.search}
            type="search"
            placeholder="Search operations"
            aria-label="Search operations"
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
            }}
          />
        </div>
      </div>
      <ul className={styles.ops}>
        {shown.map((op) => (
          <OperationRow key={op.name} op={op} target={target} />
        ))}
        {shown.length === 0 && <li className={shared.empty}>Nothing matches.</li>}
      </ul>
    </div>
  );
}

function OperationRow({ op, target }: { op: ProviderOperation; target: ProviderTarget }) {
  const resourceName = useContext(ResourceNameContext);
  const [open, setOpen] = useState(false);
  const [showing, setShowing] = useState<Record<string, unknown> | null>(null);
  const [answer, setAnswer] = useState<{ value: unknown } | null>(null);
  const direct = op.risk === 'read' && !needsInput(op);

  return (
    <li className={styles.op}>
      <div className={styles.opHead}>
        <div className={styles.opText}>
          <span className={cx(isCommand(op.name) && 'mono')}>{humanName(op.name)}</span>
          <span className={styles.opDesc}>{op.description}</span>
        </div>
        <span className={cx(shared.risk, op.risk === 'destroy' && shared.destroy)}>
          {op.request || RISK_LABEL[op.risk]}
        </span>
        {direct ? (
          <Button
            onClick={() => {
              setShowing(showing ? null : {});
            }}
          >
            {showing ? 'Hide' : 'View'}
          </Button>
        ) : (
          <Button
            variant={op.risk === 'destroy' ? 'quietDanger' : 'secondary'}
            onClick={() => {
              setOpen(true);
            }}
          >
            {op.risk === 'read' ? 'View' : 'Run'}
          </Button>
        )}
      </div>
      {showing && <ReadResult target={target} name={op.name} args={showing} />}
      {answer && (
        <div className={shared.result}>
          <Result value={answer.value} />
        </div>
      )}
      {open && (
        <OperationDialog
          op={op}
          target={target}
          confirmName={resourceName}
          onClose={() => {
            setOpen(false);
          }}
          onRead={(args) => {
            setOpen(false);
            setShowing(args);
          }}
          onDone={(value) => {
            setOpen(false);
            setAnswer({ value });
          }}
        />
      )}
    </li>
  );
}

export function ReadResult({
  target,
  name,
  args,
}: {
  target: ProviderTarget;
  name: string;
  args: Record<string, unknown>;
}) {
  const read = useProviderRead(target, name, args);
  return (
    <div className={shared.result}>
      {read.isPending && <SkeletonLines />}
      {read.error && (
        <ProviderError
          error={read.error}
          retrying={read.isFetching}
          onRetry={() => {
            void read.refetch();
          }}
        />
      )}
      {read.data !== undefined && <Result value={read.data} />}
    </div>
  );
}
