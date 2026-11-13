/**
 * Settings · General: appearance, profile, startup, notifications, data and
 * moving to a new computer.
 */
import { useState } from 'react';
import type { Profile, Settings } from '@shared/api';
import { useProfile, useSaveProfile } from '@ui/api/accounts';
import {
  useBackUpNow,
  useBackups,
  useSaveDiagnosticReport,
  useSettings,
  useUpdateSettings,
} from '@ui/api/settings';
import { Button } from '@ui/components/Button';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { QueryState } from '@ui/components/QueryState';
import {
  SelectRow,
  SettingGroup,
  SettingRow,
  SettingGroupSkeleton,
} from '@ui/components/SettingRow';
import { showToast } from '@ui/lib/toast';
import { timeAgo } from '@ui/lib/format';
import { BACKUP_CHOICES, HISTORY_CHOICES, NOTIFY_CHOICES, OPEN_AT_LOGIN_CHOICES } from './choices';
import { SectionHeader } from './SectionHeader';
import { ThemePicker } from './ThemePicker';
import { ExportTransferDialog } from './transfer/ExportTransferDialog';
import styles from './GeneralSettings.module.css';

export function GeneralSettings() {
  const profile = useProfile();
  const settings = useSettings();

  return (
    <>
      <SectionHeader title="General" />
      <QueryState query={settings} skeleton={<SettingGroupSkeleton rows={1} />}>
        {(data) => <Appearance settings={data} />}
      </QueryState>
      <QueryState query={profile} skeleton={<SettingGroupSkeleton />}>
        {(data) => (data ? <ProfileGroup profile={data} /> : null)}
      </QueryState>
      <QueryState query={settings} skeleton={<SettingGroupSkeleton cards={3} />}>
        {(data) => <GeneralGroups settings={data} />}
      </QueryState>
    </>
  );
}

function Appearance({ settings }: { settings: Settings }) {
  const update = useUpdateSettings();
  return (
    <SettingGroup title="Appearance" description="Theme">
      <ThemePicker
        value={settings.general.theme}
        onChange={(theme) => {
          update.mutate({ general: { theme } });
        }}
      />
      {update.error && <ErrorMessage error={update.error} />}
    </SettingGroup>
  );
}

function ProfileGroup({ profile }: { profile: Profile }) {
  const [name, setName] = useState(profile.name);
  const save = useSaveProfile();
  const isChanged = name !== profile.name;

  function saveIfChanged(): void {
    if (isChanged) save.mutate({ name });
  }

  return (
    <SettingGroup title="Profile" description="Stays on this computer. There’s no Hoista account.">
      <SettingRow label="Name" hint="Shown as “You” in activity and deploys">
        <input
          aria-label="Name"
          className={styles.input}
          value={name}
          onChange={(event) => {
            setName(event.target.value);
          }}
          onBlur={saveIfChanged}
        />
      </SettingRow>
      {save.error && <ErrorMessage error={save.error} />}
    </SettingGroup>
  );
}

function GeneralGroups({ settings }: { settings: Settings }) {
  const update = useUpdateSettings();
  const report = useSaveDiagnosticReport();
  const { general } = settings;
  const [isExporting, setIsExporting] = useState(false);

  return (
    <>
      <SettingGroup title="Startup and notifications">
        <SelectRow
          label="Open at login"
          hint="In the menu bar keeps Hoista running so agents can reach it."
          value={general.open_at_login}
          options={OPEN_AT_LOGIN_CHOICES}
          onChange={(openAtLogin) => {
            update.mutate({ general: { open_at_login: openAtLogin } });
          }}
        />
        <SelectRow
          label="Notify me when"
          value={general.notify}
          options={NOTIFY_CHOICES}
          onChange={(notify) => {
            update.mutate({ general: { notify } });
          }}
        />
      </SettingGroup>

      <SettingGroup title="Data">
        <SettingRow
          label="Everything Hoista knows is in ~/.hoista"
          hint="Account tokens are in your computer's keychain, not in that folder."
        />
        <Backups settings={settings} />
        <SettingRow
          label="Move to a new computer"
          hint="Saves your projects, accounts and settings to one passphrase-locked file. Open it with Hoista on the new computer."
        >
          <Button
            onClick={() => {
              setIsExporting(true);
            }}
          >
            Save file
          </Button>
        </SettingRow>
        <SettingRow
          label="Diagnostic report"
          hint="Project list, recent deploy logs and the Hoista version. Never includes tokens or secret values."
        >
          <Button
            busy={report.isPending}
            busyLabel="Saving"
            onClick={() => {
              report.mutate(undefined, {
                onSuccess: (saved) => {
                  showToast(`Report saved to ${saved.path}`);
                },
              });
            }}
          >
            Save report
          </Button>
        </SettingRow>
      </SettingGroup>
      {isExporting && (
        <ExportTransferDialog
          onClose={() => {
            setIsExporting(false);
          }}
        />
      )}
    </>
  );
}

/** Automatic backups of what Hoista knows, beside its state, never with sign-ins. */
function Backups({ settings }: { settings: Settings }) {
  const update = useUpdateSettings();
  const backups = useBackups();
  const backUp = useBackUpNow();
  const newest = backups.data?.[0];

  return (
    <>
      <SelectRow
        label="Automatic backups"
        hint={`Projects, accounts and settings, kept in ~/.hoista/backups (the newest ${String(settings.backups.keep)}). Sign-ins stay in the keychain. Restore one into a fresh Hoista with hoist backups restore.`}
        value={settings.backups.every_hours}
        options={BACKUP_CHOICES}
        onChange={(every_hours) => {
          update.mutate({ backups: { every_hours } });
        }}
      />
      <SelectRow
        label="Keep history"
        hint="Activity, finished changes and webhook deliveries older than this are removed each day. The last 30 days are always kept. Engine logs keep 14 days."
        value={settings.backups.history_days}
        options={HISTORY_CHOICES}
        onChange={(history_days) => {
          update.mutate({ backups: { history_days } });
        }}
      />
      <SettingRow
        label="Last backup"
        hint={newest ? `${timeAgo(newest.at)} · ${newest.name}` : 'None yet.'}
      >
        <Button
          busy={backUp.isPending}
          busyLabel="Backing up"
          onClick={() => {
            backUp.mutate();
          }}
        >
          Back up now
        </Button>
      </SettingRow>
      <ErrorMessage error={update.error ?? backUp.error} />
    </>
  );
}
