/**
 * The form for one provider operation, built from its argument schema: text,
 * numbers, on/off, dropdowns for fixed choices. An input object is laid out
 * as its own fields; JSON is only for what has no shape. What Hoista fills
 * in itself (project, environment, service, workspace) isn't asked for.
 * A form can start from what's set now. Operations that can't be undone
 * ask you to type the name of what they remove.
 */
import { useContext, useEffect, useRef, useState } from 'react';
import type { ProviderOperation, ProviderOperationArg, ProviderTarget } from '@shared/api';
import { useProviderRead, useRunProviderOperation } from '@ui/api/providerOperations';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ProviderError } from './ProviderError';
import { SelectField } from '@ui/components/Select';
import { FieldWordsContext, Result } from './Result';
import styles from './OperationDialog.module.css';
import shared from './provider.module.css';

interface Schema {
  type?: string;
  enum?: string[];
  items?: Schema;
  properties?: Record<string, Schema>;
  required?: string[];
  description?: string;
  /** Filled in by Hoista from the resource: `environment`, `service`… */
  'x-filled-in'?: string;
}

/** One thing the form asks for: a whole argument, or one field of an input object. */
interface Ask {
  /** `memoryGB`, or `input.memoryGB` for a field of `input`. */
  key: string;
  arg: ProviderOperationArg;
}

/** An input object with known fields is asked for field by field. */
function expandable(schema: Schema): boolean {
  return schema.type === 'object' && Object.keys(schema.properties ?? {}).length > 0;
}

/** What the form asks for, in order: required first, then the rest. */
function asksOf(op: ProviderOperation, given: (key: string) => boolean): Ask[] {
  const out: Ask[] = [];
  for (const arg of op.args) {
    if (arg.filled_in || given(arg.name)) continue;
    const schema = schemaOf(arg);
    if (expandable(schema)) {
      const required = new Set(schema.required ?? []);
      for (const [name, field] of Object.entries(schema.properties ?? {})) {
        const key = `${arg.name}.${name}`;
        if (field['x-filled-in'] || given(key)) continue;
        out.push({
          key,
          arg: { name, required: required.has(name), filled_in: null, schema: field },
        });
      }
    } else {
      out.push({ key: arg.name, arg });
    }
  }
  return out.sort((a, b) => Number(b.arg.required) - Number(a.arg.required));
}

/** What Hoista fills in, in words: `the environment and service`. */
function filledWords(op: ProviderOperation): string | null {
  const kinds = new Set<string>();
  for (const arg of op.args) {
    if (arg.filled_in) kinds.add(arg.filled_in);
    for (const field of Object.values(schemaOf(arg).properties ?? {})) {
      if (field['x-filled-in']) kinds.add(field['x-filled-in']);
    }
  }
  const list = [...kinds];
  if (list.length === 0) return null;
  return `the ${list.length > 1 ? `${list.slice(0, -1).join(', ')} and ${list.at(-1) ?? ''}` : (list[0] ?? '')}`;
}

/** Sets `input.name` style keys inside their objects. */
function put(out: Record<string, unknown>, key: string, value: unknown): void {
  const [head, ...rest] = key.split('.');
  if (!head) return;
  if (rest.length === 0) {
    out[head] = value;
    return;
  }
  const inner =
    out[head] && typeof out[head] === 'object' ? (out[head] as Record<string, unknown>) : {};
  put(inner, rest.join('.'), value);
  out[head] = inner;
}

/** A value from the provider as a form field holds it. */
function asField(value: unknown): string | boolean | string[] | undefined {
  if (typeof value === 'boolean') return value;
  if (typeof value === 'number') return String(value);
  if (typeof value === 'string') return value;
  if (Array.isArray(value) && value.every((item) => typeof item === 'string')) return value;
  return undefined;
}

/** Whether an operation is one of the provider's CLI commands, such as `supabase db push`. */
export function isCommand(name: string): boolean {
  return name.includes(' ');
}

/** `volumeInstanceBackupList` → `Volume instance backup list`; commands stay as typed. */
export function humanName(name: string): string {
  if (isCommand(name)) return name;
  const words = name
    .replace(/[-_]+/g, ' ')
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1 $2')
    .toLowerCase();
  const acronyms: Record<string, string> = {
    http: 'HTTP',
    tcp: 'TCP',
    cdn: 'CDN',
    dns: 'DNS',
    pitr: 'PITR',
    ha: 'HA',
    id: 'ID',
    ids: 'IDs',
    url: 'URL',
    ip: 'IP',
    ipv6: 'IPv6',
    s3: 'S3',
    pr: 'PR',
    api: 'API',
    mb: 'MB',
    gb: 'GB',
    cpu: 'CPU',
    vcpu: 'vCPU',
    vcpus: 'vCPUs',
  };
  const fixed = words
    .split(' ')
    .map((word) => acronyms[word] ?? word)
    .join(' ');
  return fixed.charAt(0).toUpperCase() + fixed.slice(1);
}

/** Whether running it needs anything from the person beyond what Hoista fills in. */
export function needsInput(op: ProviderOperation): boolean {
  return op.args.some((arg) => arg.required);
}

function schemaOf(arg: ProviderOperationArg): Schema {
  const schema = arg.schema;
  return schema && typeof schema === 'object' ? schema : {};
}

/** A point in time (`targetTimestamp`, `startDate`): picked with a date and time field. */
function isMoment(arg: ProviderOperationArg): boolean {
  return schemaOf(arg).type === 'string' && /(timestamp|date|time)$/i.test(arg.name);
}

function isStructured(schema: Schema): boolean {
  return (
    schema.type === 'object' ||
    (schema.type === 'array' && !(schema.items?.enum && schema.items.enum.length > 0))
  );
}

export function OperationDialog({
  op,
  target,
  title,
  preset = {},
  about,
  tryFirst,
  current,
  confirmName,
  onClose,
  onRead,
  onDone,
}: {
  op: ProviderOperation;
  target: ProviderTarget;
  /** What the person asked to do, such as `Restore this backup`; the operation's name otherwise. */
  title?: string;
  /** Arguments already chosen on screen, such as the backup to restore; not asked for again. */
  preset?: Record<string, unknown>;
  /** What it does, when the title isn't the operation's own name. */
  about?: string;
  /** Arguments of a run that changes nothing, such as a dry run: offered as Try first. */
  tryFirst?: Record<string, unknown> | null;
  /** A read whose answer fills the form with what's set now. */
  current?: string;
  /** What a change that can't be undone asks you to type: the name of what it removes. */
  confirmName?: string;
  onClose: () => void;
  /** A read: show its answer with these arguments. */
  onRead: (args: Record<string, unknown>) => void;
  /** A change ran: its answer. */
  onDone: (value: unknown) => void;
}) {
  const run = useRunProviderOperation(target);
  const trial = useRunProviderOperation(target);
  const [tried, setTried] = useState<{ value: unknown } | null>(null);
  // A dry run is its own button, so its switch isn't asked for too.
  const given = (key: string) => key in preset || (tryFirst ? key in tryFirst : false);
  const asked = asksOf(op, given);
  const filled = filledWords(op);
  const [values, setValues] = useState<Record<string, string | boolean | string[]>>({});
  const [problem, setProblem] = useState<string | null>(null);
  const [confirm, setConfirm] = useState('');
  const destroys = op.risk === 'destroy';
  const word = confirmName ?? 'delete';
  const now = useProviderRead<Record<string, unknown>>(current ? target : null, current ?? '');
  const words = useContext(FieldWordsContext);
  // What's set now fills the fields once, before you change any.
  const started = useRef(false);
  useEffect(() => {
    if (started.current || !now.data || typeof now.data !== 'object') return;
    started.current = true;
    const found: Record<string, string | boolean | string[]> = {};
    const data = now.data;
    for (const ask of asked) {
      // The same field, or one the provider labels the same way (`cpu` and `vCPUs` are both CPU).
      const label = words?.labels[ask.arg.name];
      const key =
        ask.arg.name in data
          ? ask.arg.name
          : Object.keys(data).find((name) => label !== undefined && words?.labels[name] === label);
      const value = key === undefined ? undefined : asField(data[key]);
      if (value !== undefined) found[ask.key] = value;
    }
    setValues((typed) => ({ ...found, ...typed }));
  }, [now.data, asked, words]);

  function args(): Record<string, unknown> | null {
    const out: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(preset)) put(out, key, value);
    // An input object Hoista fills in is sent even when nothing else in it is.
    for (const arg of op.args) {
      if (arg.required && !arg.filled_in && expandable(schemaOf(arg)) && !(arg.name in out)) {
        out[arg.name] = {};
      }
    }
    for (const { key, arg } of asked) {
      const schema = schemaOf(arg);
      const raw = values[key];
      if (raw === undefined || raw === '' || (Array.isArray(raw) && raw.length === 0)) {
        if (arg.required) {
          setProblem(`Fill in ${humanName(arg.name).toLowerCase()}.`);
          return null;
        }
        continue;
      }
      if (typeof raw === 'boolean' || Array.isArray(raw)) {
        put(out, key, raw);
      } else if (isStructured(schema)) {
        try {
          put(out, key, JSON.parse(raw) as unknown);
        } catch {
          setProblem(`${humanName(arg.name)} must be JSON.`);
          return null;
        }
      } else if (isMoment(arg)) {
        const at = new Date(raw);
        if (Number.isNaN(at.getTime())) {
          setProblem(`Pick a date and time for ${humanName(arg.name).toLowerCase()}.`);
          return null;
        }
        put(out, key, at.toISOString());
      } else if (schema.type === 'integer' || schema.type === 'number') {
        const number = Number(raw);
        if (Number.isNaN(number)) {
          setProblem(`${humanName(arg.name)} must be a number.`);
          return null;
        }
        put(out, key, number);
      } else {
        put(out, key, raw);
      }
    }
    setProblem(null);
    return out;
  }

  function submit(): void {
    const given = args();
    if (!given) return;
    if (op.risk === 'read') {
      onRead(given);
      return;
    }
    run.mutate(
      { name: op.name, args: given },
      {
        onSuccess: (value) => {
          onDone(value);
        },
      },
    );
  }

  function tryIt(): void {
    const given = args();
    if (!given || !tryFirst) return;
    setTried(null);
    trial.mutate(
      { name: op.name, args: { ...given, ...tryFirst } },
      {
        onSuccess: (value) => {
          setTried({ value });
        },
      },
    );
  }

  return (
    <Dialog
      title={title ?? humanName(op.name)}
      eyebrow={about ?? op.description}
      onClose={onClose}
      wide
      actions={
        <>
          <Button onClick={onClose}>Cancel</Button>
          {tryFirst && (
            <Button busy={trial.isPending} busyLabel="Trying" onClick={tryIt}>
              Try first
            </Button>
          )}
          <Button
            variant={destroys ? 'danger' : 'primary'}
            disabled={destroys && confirm.trim() !== word}
            busy={run.isPending}
            busyLabel="Running"
            onClick={submit}
          >
            {op.risk === 'read' ? 'View' : (title ?? 'Run')}
          </Button>
        </>
      }
    >
      <div className={styles.form}>
        {isCommand(op.name) && (
          <p className={shared.muted}>
            Runs <span className="mono">{op.name}</span> with Hoista’s own copy of the provider’s
            command-line tool, signed in for this run only.
          </p>
        )}
        {current && now.isPending && <p className={shared.muted}>Reading what’s set now</p>}
        {asked.length === 0 && !destroys && (
          <p className={shared.muted}>Nothing to fill in. It runs as soon as you confirm.</p>
        )}
        {asked.map(({ key, arg }) => (
          <ArgField
            key={key}
            arg={arg}
            value={values[key]}
            onChange={(value) => {
              setValues((typed) => ({ ...typed, [key]: value }));
            }}
          />
        ))}
        {filled && <p className={shared.muted}>Hoista fills in {filled} from this resource.</p>}
        {destroys && (
          <label className={styles.confirm}>
            <span>
              This can’t be undone at the provider. Type <b className="mono">{word}</b> to confirm.
            </span>
            <input
              className={styles.input}
              value={confirm}
              onChange={(event) => {
                setConfirm(event.target.value);
              }}
            />
          </label>
        )}
        {problem && <p className={styles.problem}>{problem}</p>}
        {tried && (
          <div className={shared.result}>
            <p className={shared.muted}>What a real run would do. Nothing changed.</p>
            <Result value={tried.value} />
          </div>
        )}
        <ProviderError error={trial.error} />
        <ProviderError error={run.error} onRetry={submit} retrying={run.isPending} />
      </div>
    </Dialog>
  );
}

function ArgField({
  arg,
  value,
  onChange,
}: {
  arg: ProviderOperationArg;
  value: string | boolean | string[] | undefined;
  onChange: (value: string | boolean | string[]) => void;
}) {
  const schema = schemaOf(arg);
  const words = useContext(FieldWordsContext);
  const unit = words?.units[arg.name];
  const label = `${words?.labels[arg.name] ?? humanName(arg.name)}${unit ? ` (${unit})` : ''}${arg.required ? '' : ' (optional)'}`;
  const hint = schema.description ? (
    <span className={styles.fieldHint}>{schema.description}</span>
  ) : null;
  if (schema.enum && schema.enum.length > 0) {
    const options = [
      ...(arg.required ? [] : [{ value: '', label: 'Not set' }]),
      ...schema.enum.map((item) => ({ value: item, label: item })),
    ];
    return (
      <SelectField
        label={label}
        value={typeof value === 'string' ? value : arg.required ? (schema.enum[0] ?? '') : ''}
        options={options}
        onChange={onChange}
      />
    );
  }
  if (schema.type === 'array' && schema.items?.enum) {
    const chosen = Array.isArray(value) ? value : [];
    return (
      <fieldset className={styles.choices}>
        <legend>{label}</legend>
        {schema.items.enum.map((item) => (
          <label key={item} className={styles.choice}>
            <input
              type="checkbox"
              checked={chosen.includes(item)}
              onChange={(event) => {
                onChange(
                  event.target.checked ? [...chosen, item] : chosen.filter((x) => x !== item),
                );
              }}
            />
            <span className="mono">{item}</span>
          </label>
        ))}
      </fieldset>
    );
  }
  if (schema.type === 'boolean') {
    return (
      <label className={styles.choice}>
        <input
          type="checkbox"
          checked={value === true}
          onChange={(event) => {
            onChange(event.target.checked);
          }}
        />
        <span>{label}</span>
      </label>
    );
  }
  const structured = isStructured(schema);
  return (
    <label className={styles.field}>
      <span className={styles.fieldLabel}>{label}</span>
      {hint}
      {structured ? (
        <textarea
          className={cx(styles.input, styles.json)}
          rows={4}
          placeholder={placeholder(schema)}
          value={typeof value === 'string' ? value : ''}
          onChange={(event) => {
            onChange(event.target.value);
          }}
        />
      ) : (
        <input
          className={styles.input}
          type={
            isMoment(arg)
              ? 'datetime-local'
              : schema.type === 'integer' || schema.type === 'number'
                ? 'number'
                : 'text'
          }
          value={typeof value === 'string' ? value : ''}
          onChange={(event) => {
            onChange(event.target.value);
          }}
        />
      )}
    </label>
  );
}

/** A starting shape for JSON fields: the object's fields, required ones first. */
function placeholder(schema: Schema): string {
  if (schema.type === 'array') return 'A JSON list, such as ["a", "b"]';
  if (schema.type !== 'object' || !schema.properties) return 'JSON, such as true, 10 or "text"';
  const keys = Object.keys(schema.properties);
  const required = new Set(schema.required ?? []);
  keys.sort((a, b) => Number(required.has(b)) - Number(required.has(a)));
  return `{ ${keys
    .slice(0, 6)
    .map((key) => `"${key}": …`)
    .join(', ')}${keys.length > 6 ? ', …' : ''} }`;
}

function cx(...names: (string | false | undefined)[]): string {
  return names.filter(Boolean).join(' ');
}
