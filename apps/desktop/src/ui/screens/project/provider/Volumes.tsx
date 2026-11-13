import { useState } from 'react';
import type { ProviderOperation, ProviderTarget } from '@shared/api';
import { useProviderDetails } from '@ui/api/projects';
import { useProviderRead } from '@ui/api/providerOperations';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { SkeletonLines, SkeletonRows } from '@ui/components/Skeleton';
import { cx } from '@ui/lib/cx';
import { regionName } from '@ui/lib/regions';
import { RowMenu } from '@ui/components/RowMenu';
import styles from './Volumes.module.css';
import shared from './provider.module.css';
import { DAY_SECONDS } from '@shared/time';
import { formatMegabytes } from '@ui/lib/format';
import { ActionBar, ActionButton, type RunSpec, Runner } from './Runner';

interface Backup {
  id: string;
  name?: string | null;
  createdAt?: string;
  expiresAt?: string | null;
  usedMB?: number | null;
}

export function size(mb: number | null | undefined): string {
  return mb === null || mb === undefined ? '—' : formatMegabytes(mb);
}

function when(at: string | null | undefined): string {
  if (!at) return '—';
  return new Date(at).toLocaleString('en-GB', {
    day: 'numeric',
    month: 'short',
    hour: '2-digit',
    minute: '2-digit',
  });
}

/** The resource's volumes, each with its backups and what you can do to them. */
export function Volumes({
  resourceId,
  target,
  byName,
}: {
  resourceId: string;
  target: ProviderTarget;
  byName: Map<string, ProviderOperation>;
}) {
  const details = useProviderDetails(resourceId);
  const volumes =
    (
      details.data as
        | {
            volumes?: {
              id: string;
              name: string;
              mount_path: string | null;
              instance_id?: string | null;
              used_mb?: number | null;
              size_mb?: number | null;
              region?: string | null;
            }[];
          }
        | undefined
    )?.volumes ?? [];
  if (details.isPending) {
    return (
      <section className={shared.card}>
        <div className={shared.cardBody} style={{ paddingTop: 16 }}>
          <SkeletonLines />
        </div>
      </section>
    );
  }
  if (volumes.length === 0) {
    return <p className={shared.muted}>This resource has no volumes.</p>;
  }
  return (
    <>
      {volumes.map((volume) => (
        <Volume key={volume.id} volume={volume} target={target} byName={byName} />
      ))}
    </>
  );
}

function Volume({
  volume,
  target,
  byName,
}: {
  volume: {
    id: string;
    name: string;
    mount_path: string | null;
    instance_id?: string | null;
    used_mb?: number | null;
    size_mb?: number | null;
    region?: string | null;
  };
  target: ProviderTarget;
  byName: Map<string, ProviderOperation>;
}) {
  const instance = volume.instance_id ?? null;
  const [running, setRunning] = useState<RunSpec | null>(null);
  const backups = useProviderRead<Backup[] | { edges?: { node: Backup }[] }>(
    instance ? target : null,
    'volumeInstanceBackupList',
    { volumeInstanceId: instance },
  );
  const list: Backup[] = Array.isArray(backups.data)
    ? backups.data
    : (backups.data?.edges ?? []).map((edge) => edge.node);
  const op = (name: string) => byName.get(name);
  const schedules = useProviderRead<{ kind: string; retentionSeconds?: number | null }[]>(
    instance ? target : null,
    'volumeInstanceBackupScheduleList',
    { volumeInstanceId: instance },
  );
  const backup = op('volumeInstanceBackupCreate');
  const pointInTime = op('volumeInstancePITRRestore');
  const remove = op('volumeInstanceBackupDelete');
  const keep = op('volumeInstanceBackupLock');
  const schedule = op('volumeInstanceBackupScheduleUpdate');
  const resize = op('volumeInstanceUpdate');
  const restore = op('volumeInstanceBackupRestore');
  const used =
    volume.used_mb !== null && volume.used_mb !== undefined && volume.size_mb
      ? Math.min(volume.used_mb / volume.size_mb, 1)
      : null;
  return (
    <>
      <section className={shared.card}>
        <div className={shared.cardHead}>
          <h4>{volume.name}</h4>
          <span className={shared.cardAbout}>
            Volume{volume.mount_path ? ` · mounted at ${volume.mount_path}` : ''}
          </span>
          <span className={styles.cardActions}>
            {resize && (
              <ActionButton
                op={resize}
                label="Resize"
                target={target}
                preset={{ volumeId: volume.id }}
              />
            )}
            {backup && instance && (
              <ActionButton
                op={backup}
                label="Back up now"
                target={target}
                preset={{ volumeInstanceId: instance }}
              />
            )}
          </span>
        </div>
        <dl className={styles.volumeFacts}>
          <div>
            <dt>Used</dt>
            <dd>
              {size(volume.used_mb)}
              {volume.size_mb ? ` of ${size(volume.size_mb)}` : ''}
            </dd>
          </div>
          {used !== null && (
            <div className={styles.meterCell}>
              <span className={styles.meter}>
                <i style={{ width: `${(used * 100).toFixed(0)}%` }} />
              </span>
            </div>
          )}
          {volume.region && (
            <div>
              <dt>Region</dt>
              <dd>{regionName(volume.region)}</dd>
            </div>
          )}
        </dl>
      </section>
      {instance && (
        <section className={shared.card}>
          <div className={shared.cardHead}>
            <h4>Backups</h4>
            <span className={shared.cardAbout}>{scheduleText(schedules.data)}</span>
            <span className={styles.cardActions}>
              <ActionBar
                target={target}
                actions={[
                  ...(schedule
                    ? [
                        {
                          op: schedule,
                          label: 'Backup schedule',
                          preset: { volumeInstanceId: instance },
                        },
                      ]
                    : []),
                  ...(pointInTime
                    ? [
                        {
                          op: pointInTime,
                          label: 'Restore to a time',
                          preset: { volumeInstanceId: instance },
                          confirmName: volume.name,
                        },
                      ]
                    : []),
                ]}
              />
            </span>
          </div>
          {backups.isPending && <SkeletonRows count={3} columns={5} />}
          {backups.error && (
            <div className={shared.cardBody}>
              <ErrorMessage error={backups.error} />
            </div>
          )}
          {backups.data !== undefined && list.length === 0 && (
            <p className={cx(shared.muted, shared.cardBody)}>No backups yet.</p>
          )}
          {list.length > 0 && (
            <div className={styles.backups}>
              <div className={cx(styles.backupRow, styles.backupHead)}>
                <span>Taken</span>
                <span>Name</span>
                <span>Size</span>
                <span>Expires</span>
                <span />
              </div>
              {list.map((item) => (
                <div key={item.id} className={styles.backupRow}>
                  <span>{when(item.createdAt)}</span>
                  <span className={styles.mutedCell}>{item.name ?? '—'}</span>
                  <span className="mono">{size(item.usedMB)}</span>
                  <span className={styles.mutedCell}>
                    {item.expiresAt ? when(item.expiresAt) : 'Never'}
                  </span>
                  <span className={styles.rowAction}>
                    <RowMenu
                      label={`More for the backup from ${when(item.createdAt)}`}
                      items={[
                        ...(keep && item.expiresAt ? [{ op: keep, label: 'Keep forever' }] : []),
                        ...(restore ? [{ op: restore, label: 'Restore' }] : []),
                        ...(remove ? [{ op: remove, label: 'Delete' }] : []),
                      ].map(({ op: action, label }) => ({
                        label,
                        danger: action.risk === 'destroy',
                        onSelect: () => {
                          setRunning({
                            op: action,
                            label: `${label} backup`,
                            preset: { volumeInstanceId: instance, volumeInstanceBackupId: item.id },
                            confirmName: volume.name,
                          });
                        },
                      }))}
                    />
                  </span>
                </div>
              ))}
              <p className={styles.backupNote}>
                Restoring replaces what’s on {volume.name} now. Hoista asks you first.
              </p>
            </div>
          )}
        </section>
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
    </>
  );
}

/** `Daily and weekly · kept up to 28 days`, from the provider's backup schedules. */
function scheduleText(
  schedules: { kind: string; retentionSeconds?: number | null }[] | undefined,
): string {
  if (!schedules) return 'Kept by the provider';
  if (schedules.length === 0) return 'No automatic backups';
  const kinds = schedules.map((item) => item.kind.toLowerCase());
  const list =
    kinds.length > 1
      ? `${kinds.slice(0, -1).join(', ')} and ${kinds.at(-1) ?? ''}`
      : (kinds[0] ?? '');
  const days = Math.max(...schedules.map((item) => item.retentionSeconds ?? 0)) / DAY_SECONDS;
  const text = `${list.charAt(0).toUpperCase()}${list.slice(1)}`;
  return days > 0 ? `${text} · kept up to ${String(Math.round(days))} days` : text;
}
