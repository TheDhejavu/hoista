/**
 * Settings · Safety: what the secret check does before every deploy Hoista
 * sends (always on, so it reads as facts, not choices), and how a project's
 * build code is kept away from the rest of this computer.
 */
import type { Settings } from '@shared/api';
import { useState } from 'react';
import { useSettings, useUpdateSettings } from '@ui/api/settings';
import { Button } from '@ui/components/Button';
import { StopEverythingDialog } from './remote/RemoteDialogs';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { QueryState } from '@ui/components/QueryState';
import {
  FixedValue,
  SelectRow,
  SettingGroup,
  SettingRow,
  SettingGroupSkeleton,
} from '@ui/components/SettingRow';
import { ON_OFF_CHOICES } from './choices';
import { SectionHeader } from './SectionHeader';

export function SafetySettings() {
  const settings = useSettings();
  return (
    <>
      <SectionHeader title="Safety" />
      <SettingGroup
        title="The secret check"
        description="Before code leaves this computer, Hoista looks for keys that would end up somewhere they could be read."
        note={
          <>
            <b>Not covered</b>
            Deploys a provider starts on its own, such as building a push to GitHub. Those never
            pass through Hoista. Deploy through Hoista, or turn on the provider’s own checks.
          </>
        }
      >
        <SettingRow
          label="Known keys"
          // provider-name-ok: these are the kinds of key the check knows, not providers.
          hint="Stripe, OpenAI, AWS, GitHub, Supabase and npm keys, private keys, and database URLs with a password in them."
        >
          <FixedValue>Always on</FixedValue>
        </SettingRow>
        <SettingRow
          label="Your saved secrets"
          hint="The exact values you keep in Hoista, if one turns up in the code or the build."
        >
          <FixedValue>Always on</FixedValue>
        </SettingRow>
        <SettingRow
          label="Where it looks"
          hint="The code changes being sent, and every file the build produces."
        >
          <FixedValue>Changes and build output</FixedValue>
        </SettingRow>
        <SettingRow
          label="When it finds one"
          hint="Nothing is released. The agent is told which file and what kind of key, never the key itself."
        >
          <FixedValue>Stops the deploy</FixedValue>
        </SettingRow>
      </SettingGroup>
      <QueryState query={settings} skeleton={<SettingGroupSkeleton />}>
        {(data) => <BuildIsolation safety={data.safety} />}
      </QueryState>
      <CutOff />
    </>
  );
}

/** Builds run the project's own install and build scripts, which Hoista can't vouch for. */
function BuildIsolation({ safety }: { safety: Settings['safety'] }) {
  const update = useUpdateSettings();
  const isolated = safety.builds_isolated_by !== null;

  return (
    <SettingGroup
      title="Builds"
      description="A build runs the project’s own install and build scripts, from packages Hoista can’t vouch for."
    >
      <SettingRow
        label="Kept away from this computer"
        hint={
          isolated
            ? 'Build code sees only its own copy of the project and a package cache. Not your home folder, Hoista’s keys or the keychain. It can still reach the internet to install packages.'
            : (safety.builds_not_isolated_because ?? 'Builds can’t be isolated here.')
        }
      >
        <FixedValue>
          {isolated ? `On · ${safety.builds_isolated_by ?? ''}` : 'Not available'}
        </FixedValue>
      </SettingRow>
      {!isolated && (
        <SelectRow
          label="Build without isolation"
          hint="Build code would run with your access to this computer. Hoista asks you to confirm it’s you before turning this on. Until then, builds here are stopped."
          value={safety.build_without_isolation}
          options={ON_OFF_CHOICES}
          onChange={(on) => {
            update.mutate({ safety: { build_without_isolation: on } });
          }}
        />
      )}
      <ErrorMessage error={update.error} />
    </SettingGroup>
  );
}

/** For when something looks wrong: every agent cut off at once. */
function CutOff() {
  const [open, setOpen] = useState(false);
  return (
    <SettingGroup title="If something looks wrong" danger>
      <SettingRow
        label="Cut off every agent"
        hint="Every agent loses access and agent access is paused. Your projects keep running."
      >
        <Button
          variant="danger"
          onClick={() => {
            setOpen(true);
          }}
        >
          Cut off
        </Button>
      </SettingRow>
      {open && (
        <StopEverythingDialog
          onClose={() => {
            setOpen(false);
          }}
        />
      )}
    </SettingGroup>
  );
}
