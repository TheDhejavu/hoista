/**
 * Paste a `.env` file's lines to set many variables at once on a watched
 * project's provider: on one service, or shared by every service in the project.
 * Values go straight to the provider and are never kept. Redeploying can wait,
 * so a batch of changes lands in one deploy.
 */
import { runsCode } from '../resourceKinds';
import { showToast } from '@ui/lib/toast';
import { useId, useState } from 'react';
import type { Resource } from '@shared/api';
import { providerEntry, serviceName } from '@shared/providers';
import { useSetVariables } from '@ui/api/projects';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { SelectField } from '@ui/components/Select';
import { useProjectNames } from '@ui/lib/useProjectNames';
import styles from './VariableDialog.module.css';

const NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;

/** `KEY=value` lines; `export`, quotes and comments allowed, as in a `.env` file. */
export function parseEnv(text: string): { values: Record<string, string>; bad: number[] } {
  const values: Record<string, string> = {};
  const bad: number[] = [];
  text.split(/\r?\n/).forEach((raw, index) => {
    const line = raw.trim();
    if (!line || line.startsWith('#')) return;
    const body = line.startsWith('export ') ? line.slice(7).trim() : line;
    const at = body.indexOf('=');
    const name = at > 0 ? body.slice(0, at).trim() : '';
    if (!NAME.test(name)) {
      bad.push(index + 1);
      return;
    }
    let value = body.slice(at + 1).trim();
    if (
      value.length >= 2 &&
      ((value.startsWith('"') && value.endsWith('"')) ||
        (value.startsWith("'") && value.endsWith("'")))
    ) {
      value = value.slice(1, -1);
    }
    if (!value) {
      bad.push(index + 1);
      return;
    }
    values[name] = value;
  });
  return { values, bad };
}

export function PasteVariablesDialog({
  projectId,
  resources,
  provider,
  onClose,
}: {
  projectId: string;
  resources: Resource[];
  provider: string;
  onClose: () => void;
}) {
  const textId = useId();
  const projectName = useProjectNames();
  const save = useSetVariables(projectId);
  const [text, setText] = useState('');
  const [target, setTarget] = useState(
    resources.find((resource) => runsCode(resource.kind))?.id ?? resources[0]?.id ?? '',
  );
  const [shared, setShared] = useState(false);
  const [redeploy, setRedeploy] = useState(true);
  const chosen = resources.find((resource) => resource.id === target);
  const providerName = chosen ? serviceName(chosen.provider) : provider;
  const how = chosen ? providerEntry(chosen.provider)?.variables : undefined;
  const sharing = how?.shared === true;
  const redeploys = how?.redeploys === true;
  const { values, bad } = parseEnv(text);
  const count = Object.keys(values).length;

  return (
    <Dialog
      eyebrow={`${projectName(projectId)} · Environment`}
      title="Paste many variables"
      wide
      onClose={onClose}
      actions={
        <>
          <Button large onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            large
            disabled={count === 0 || bad.length > 0}
            busy={save.isPending}
            busyLabel="Saving"
            onClick={() => {
              save.mutate(
                {
                  resourceId: target,
                  variables: values,
                  shared: shared && sharing,
                  redeploy: redeploy && redeploys,
                },
                {
                  onSuccess: () => {
                    showToast(
                      `Saved ${String(count)} ${count === 1 ? 'variable' : 'variables'}${redeploy && redeploys ? '; redeploying' : ''}`,
                    );
                    onClose();
                  },
                },
              );
            }}
          >
            {count === 0
              ? 'Save'
              : `Save ${String(count)} ${count === 1 ? 'variable' : 'variables'}${redeploy && redeploys ? ' and redeploy' : ''}`}
          </Button>
        </>
      }
    >
      <div className={styles.form}>
        <SelectField
          label="Set them on"
          value={shared && sharing ? '' : target}
          options={[
            ...resources.map((resource) => ({
              value: resource.id,
              label: resource.name,
              group: serviceName(resource.provider),
            })),
            ...(sharing ? [{ value: '', label: 'Shared with every service' }] : []),
          ]}
          onChange={(next) => {
            setShared(next === '');
            if (next) setTarget(next);
          }}
        />
        <div className={styles.field}>
          <label htmlFor={textId} className={styles.label}>
            Lines from a .env file
          </label>
          <textarea
            id={textId}
            className={styles.input}
            rows={8}
            value={text}
            spellCheck={false}
            autoComplete="off"
            placeholder={'RESEND_API_KEY=re_…\nAUTH_MODE=strict'}
            style={{ fontFamily: 'var(--font-mono)', resize: 'vertical' }}
            onChange={(event) => {
              setText(event.target.value);
            }}
          />
          <p className={styles.hint}>
            {bad.length > 0 ? (
              <span className={styles.warning}>
                {bad.length === 1 ? 'Line' : 'Lines'} {bad.join(', ')}{' '}
                {bad.length === 1 ? 'isn’t' : 'aren’t'} NAME=value. Fix or remove{' '}
                {bad.length === 1 ? 'it' : 'them'}.
              </span>
            ) : (
              `Sent straight to ${providerName}; existing variables with the same names are replaced. Hoista doesn't keep the values.`
            )}
          </p>
        </div>
        {redeploys && (
          <label className={styles.check}>
            <input
              type="checkbox"
              checked={redeploy}
              onChange={(event) => {
                setRedeploy(event.target.checked);
              }}
            />
            Redeploy now to apply them
          </label>
        )}
        <ErrorMessage error={save.error} />
      </div>
    </Dialog>
  );
}
