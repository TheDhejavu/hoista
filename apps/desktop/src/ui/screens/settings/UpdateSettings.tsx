/**
 * Settings · Updates: the installed version, any update waiting with what's
 * new and a button to install it, and how updates arrive.
 */
import { useState } from 'react';
import { Loader2 } from 'lucide-react';
import type { Settings } from '@shared/api';
import {
  useCheckForUpdates,
  useInstallUpdate,
  useSettings,
  useUpdateSettings,
} from '@ui/api/settings';
import { Button } from '@ui/components/Button';
import { Dialog, DialogText } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { QueryState } from '@ui/components/QueryState';
import {
  SelectRow,
  SettingGroup,
  SettingRow,
  SettingGroupSkeleton,
} from '@ui/components/SettingRow';
import { timeAgo } from '@ui/lib/format';
import { CHANNEL_CHOICES, ON_OFF_CHOICES } from './choices';
import { SectionHeader } from './SectionHeader';
import styles from './UpdateSettings.module.css';

export function UpdateSettings() {
  const settings = useSettings();

  return (
    <>
      <SectionHeader title="Updates" />
      <QueryState query={settings} skeleton={<SettingGroupSkeleton cards={2} />}>
        {(data) => <UpdateGroups updates={data.updates} />}
      </QueryState>
    </>
  );
}

function UpdateGroups({ updates }: { updates: Settings['updates'] }) {
  const update = useUpdateSettings();
  const check = useCheckForUpdates();
  const [isInstalling, setInstalling] = useState(false);
  const hasUpdate =
    updates.latest_version !== null && updates.latest_version !== updates.current_version;

  return (
    <>
      {hasUpdate && (
        <div className={styles.waiting}>
          <div className={styles.waitingText}>
            <strong>Version {updates.latest_version} is ready</strong>
            <ul className={styles.notes}>
              {updates.latest_notes.map((note) => (
                <li key={note}>{note}</li>
              ))}
            </ul>
          </div>
          <Button
            variant="primary"
            onClick={() => {
              setInstalling(true);
            }}
          >
            Restart and update
          </Button>
        </div>
      )}

      <SettingGroup title="Hoista Desktop" note="Updates never touch your projects or tokens.">
        <SettingRow
          label={`Version ${updates.current_version}`}
          hint={
            !updates.can_update
              ? 'This copy can’t update itself. New versions are on hoista.dev.'
              : hasUpdate
                ? `v${updates.latest_version ?? ''} is ready to install.`
                : updates.checked_at
                  ? `You're up to date. Checked ${timeAgo(updates.checked_at)}.`
                  : 'Not checked for updates yet.'
          }
        >
          {updates.can_update && !hasUpdate && (
            <Button
              disabled={check.isPending}
              onClick={() => {
                check.mutate();
              }}
            >
              {check.isPending ? 'Checking' : 'Check now'}
            </Button>
          )}
        </SettingRow>
        {updates.can_update && (
          <>
            <SelectRow
              label="Update channel"
              hint="Beta gets new features first."
              value={updates.channel}
              options={CHANNEL_CHOICES}
              onChange={(channel) => {
                update.mutate({ updates: { channel } });
              }}
            />
            <SelectRow
              label="Check for updates automatically"
              value={updates.auto_check}
              options={ON_OFF_CHOICES}
              onChange={(autoCheck) => {
                update.mutate({ updates: { auto_check: autoCheck } });
              }}
            />
          </>
        )}
      </SettingGroup>
      <ErrorMessage error={check.error ?? update.error} />

      {isInstalling && updates.latest_version && (
        <InstallDialog
          version={updates.latest_version}
          onClose={() => {
            setInstalling(false);
          }}
        />
      )}
    </>
  );
}

function InstallDialog({ version, onClose }: { version: string; onClose: () => void }) {
  const install = useInstallUpdate();
  const working = install.isPending;

  return (
    <Dialog
      title={`Update to ${version}?`}
      eyebrow="Settings · Updates"
      onClose={working ? () => undefined : onClose}
      actions={
        <>
          <Button variant="ghost" disabled={working} onClick={onClose}>
            Later
          </Button>
          <Button
            variant="primary"
            busy={working}
            disabled={working}
            onClick={() => {
              install.mutate();
            }}
          >
            {working ? 'Updating' : 'Restart and update'}
          </Button>
        </>
      }
    >
      {working ? (
        <p className={styles.working} aria-live="polite">
          <Loader2 size={13} className={styles.spin} aria-hidden="true" />
          Downloading it and checking it’s signed by Hoista. Hoista restarts by itself when it’s
          ready.
        </p>
      ) : (
        <DialogText>
          <p>Hoista downloads the update, checks its signature, then closes and opens again.</p>
          <ul>
            <li>Your projects keep running at their providers.</li>
            <li>Your remote keeps running the version it has.</li>
          </ul>
        </DialogText>
      )}
      <ErrorMessage error={install.error} />
    </Dialog>
  );
}
