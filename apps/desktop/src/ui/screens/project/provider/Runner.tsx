import { createContext, useContext, useState } from 'react';
import type { ProviderOperation, ProviderTarget } from '@shared/api';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { OperationDialog, needsInput } from './OperationDialog';
import { Result } from './Result';
import { RowMenu } from '@ui/components/RowMenu';
import { EngineError } from '@shared/engine';
import styles from './provider.module.css';
import { ReadResult } from './OperationsExplorer';

/** What one action runs: an operation, its preset arguments and how it's worded. */
export interface RunSpec {
  op: ProviderOperation;
  label: string;
  about?: string | undefined;
  preset?: Record<string, unknown>;
  tryFirst?: Record<string, unknown> | null;
  /** A read that fills the form with what's set now. */
  current?: string | undefined;
  /** What a delete asks you to type: the name of what it removes. */
  confirmName?: string | undefined;
}

/** The resource's name, which a delete with no row of its own asks you to type. */
export const ResourceNameContext = createContext<string | undefined>(undefined);

/**
 * Runs one action: a read with nothing to fill in shows its answer straight
 * away; anything else opens its form, then shows what came back.
 */
export function Runner({
  spec,
  target,
  onClose,
}: {
  spec: RunSpec;
  target: ProviderTarget;
  onClose: () => void;
}) {
  const resourceName = useContext(ResourceNameContext);
  const preset = spec.preset ?? {};
  const direct = spec.op.risk === 'read' && !needsInput(spec.op);
  const [reading, setReading] = useState<Record<string, unknown> | null>(direct ? preset : null);
  const [done, setDone] = useState<{ value: unknown } | null>(null);
  if (reading) {
    return (
      <Dialog
        title={spec.label}
        eyebrow={spec.about ?? spec.op.description}
        wide
        onClose={onClose}
        actions={<Button onClick={onClose}>Close</Button>}
      >
        <ReadResult target={target} name={spec.op.name} args={reading} />
      </Dialog>
    );
  }
  if (done) return <DoneDialog label={spec.label} value={done.value} onClose={onClose} />;
  return (
    <OperationDialog
      op={spec.op}
      target={target}
      title={spec.label}
      about={spec.about}
      preset={preset}
      tryFirst={spec.tryFirst ?? null}
      current={spec.current}
      confirmName={spec.confirmName ?? resourceName}
      onClose={onClose}
      onRead={setReading}
      onDone={(value) => {
        setDone({ value });
      }}
    />
  );
}

/** A button for one action; it opens the form, and shows what came back. */
export function ActionButton({
  target,
  variant,
  ...spec
}: RunSpec & { target: ProviderTarget; variant?: 'secondary' | 'quietDanger' }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button
        variant={variant ?? (spec.op.risk === 'destroy' ? 'quietDanger' : 'secondary')}
        title={spec.about ?? spec.op.description}
        onClick={() => {
          setOpen(true);
        }}
      >
        {spec.label}
      </Button>
      {open && (
        <Runner
          spec={spec}
          target={target}
          onClose={() => {
            setOpen(false);
          }}
        />
      )}
    </>
  );
}

/**
 * A section's actions: the first few as buttons, the rest and anything that
 * can't be undone behind ⋮, as on the providers' own dashboards.
 */
export function ActionBar({ actions, target }: { actions: RunSpec[]; target: ProviderTarget }) {
  const [running, setRunning] = useState<RunSpec | null>(null);
  // Anything that takes it offline for a while (pause, stop, restart) is
  // secondary, like on the providers' own pages: it waits behind ⋮ with the rest.
  const disruptive = (action: RunSpec) =>
    action.op.risk === 'change' && /\b(pause|stop|suspend|restart|reboot)\b/i.test(action.label);
  const safe = actions.filter((action) => action.op.risk !== 'destroy' && !disruptive(action));
  const shown = safe.slice(0, 3);
  const more = [
    ...safe.slice(3),
    ...actions.filter(disruptive),
    ...actions.filter((action) => action.op.risk === 'destroy'),
  ];
  if (actions.length === 0) return null;
  return (
    <div className={styles.actions}>
      {shown.map((action) => (
        <ActionButton key={`${action.op.name}-${action.label}`} {...action} target={target} />
      ))}
      {more.length > 0 && (
        <RowMenu
          label="More actions"
          items={more.map((action) => ({
            label: action.label,
            danger: action.op.risk === 'destroy',
            onSelect: () => {
              setRunning(action);
            },
          }))}
        />
      )}
      {running && (
        <Runner
          spec={running}
          target={target}
          onClose={() => {
            setRunning(null);
          }}
        />
      )}
    </div>
  );
}

function DoneDialog({
  label,
  value,
  onClose,
}: {
  label: string;
  value: unknown;
  onClose: () => void;
}) {
  return (
    <Dialog
      title={`${label}: done`}
      onClose={onClose}
      actions={<Button onClick={onClose}>Close</Button>}
    >
      <Result value={value} />
    </Dialog>
  );
}

/**
 * Why a read came back empty. A provider that turns a read down in its own
 * words (a feature not on this plan, nothing set up yet) says so quietly;
 * anything else is an error.
 */
export function Refusal({ error }: { error: Error }) {
  const said = error instanceof EngineError && error.code === 'provider.rejected';
  const words = error.message.replace(/^The provider rejected the operation: /, '');
  if (!said || words === error.message) return <ErrorMessage error={error} />;
  return <p className={styles.cardAbout}>{words}</p>;
}
