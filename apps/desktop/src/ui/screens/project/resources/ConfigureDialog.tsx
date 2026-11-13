import { useState } from 'react';
import { useProviderChange } from '@ui/api/deploys';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { SelectField } from '@ui/components/Select';
import styles from './ResourceScreen.module.css';
import type { ProviderConfig } from '@shared/api';

const RESTART_OPTIONS = [
  { value: 'on_failure', label: 'On failure' },
  { value: 'always', label: 'Always' },
  { value: 'never', label: 'Never' },
];

/** Change how it builds and runs; the provider redeploys with the new settings. */
export function ConfigureDialog({
  resourceId,
  name,
  provider,
  config,
  onClose,
}: {
  resourceId: string;
  name: string;
  provider: string;
  config: ProviderConfig;
  onClose: () => void;
}) {
  const change = useProviderChange();
  const [values, setValues] = useState({
    start_command: config.start_command ?? '',
    build_command: config.build_command ?? '',
    root_directory: config.root_directory ?? '',
    healthcheck_path: config.healthcheck_path ?? '',
    cron_schedule: config.cron_schedule ?? '',
    replicas: config.replicas === null ? '' : String(config.replicas),
    region: config.region ?? '',
  });
  const [restart, setRestart] = useState((config.restart_policy ?? 'on_failure').toLowerCase());
  const field = (key: keyof typeof values, label: string, placeholder: string) => (
    <label className={styles.field}>
      <span>{label}</span>
      <input
        className={styles.input}
        value={values[key]}
        placeholder={placeholder}
        onChange={(event) => {
          setValues((current) => ({ ...current, [key]: event.target.value }));
        }}
      />
    </label>
  );
  const replicasOk = values.replicas === '' || /^\d+$/.test(values.replicas);

  function save(): void {
    const changed: Record<string, unknown> = {};
    const before: Record<string, string> = {
      start_command: config.start_command ?? '',
      build_command: config.build_command ?? '',
      root_directory: config.root_directory ?? '',
      healthcheck_path: config.healthcheck_path ?? '',
      cron_schedule: config.cron_schedule ?? '',
      replicas: config.replicas === null ? '' : String(config.replicas),
      region: config.region ?? '',
    };
    for (const [key, value] of Object.entries(values)) {
      if (value !== before[key] && value !== '') {
        changed[key] = key === 'replicas' ? Number(value) : value;
      }
    }
    if (restart !== (config.restart_policy ?? 'on_failure').toLowerCase())
      changed.restart_policy = restart;
    if (Object.keys(changed).length === 0) {
      onClose();
      return;
    }
    change.mutate(
      { resourceId, body: { action: 'configure', config: changed } },
      { onSuccess: onClose },
    );
  }

  return (
    <Dialog
      title={`How ${name} builds and runs`}
      eyebrow={`Saved at ${provider}; it redeploys with the new settings.`}
      onClose={onClose}
      wide
      actions={
        <>
          <Button large onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            large
            disabled={!replicasOk}
            busy={change.isPending}
            busyLabel="Saving"
            onClick={save}
          >
            Save and redeploy
          </Button>
        </>
      }
    >
      <div className={styles.form}>
        {field('start_command', 'Start command', 'Detected by ' + provider)}
        {field('build_command', 'Build command', 'Detected by ' + provider)}
        {field('root_directory', 'Root folder', '/')}
        {field('healthcheck_path', 'Health check path', '/health')}
        {field('cron_schedule', 'Schedule (cron)', 'Only for jobs, e.g. 0 3 * * *')}
        {field('replicas', 'Copies', '1')}
        {field('region', 'Region', 'Default region')}
        <SelectField
          label="Restarts"
          value={restart}
          options={RESTART_OPTIONS}
          onChange={setRestart}
        />
        {!replicasOk && <p className={styles.problem}>Copies must be a whole number.</p>}
        <ErrorMessage error={change.error} />
      </div>
    </Dialog>
  );
}
