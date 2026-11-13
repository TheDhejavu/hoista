import { RepositoryList } from './RepositoryList';
import { serviceName } from '@shared/providers';
import { useMemo } from 'react';
import type { Account } from '@shared/api';
import { useEmptyProviderProjects, useProviderProjects } from '@ui/api/imports';
import { Button } from '@ui/components/Button';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { SettingGroup, SettingRow } from '@ui/components/SettingRow';
import { SkeletonRows } from '@ui/components/Skeleton';
import { showToast } from '@ui/lib/toast';
import { ProjectPicker } from '@ui/screens/projects/ProjectPicker';
import { useApplyPicker, usePicker } from '@ui/screens/projects/usePicker';
import styles from './ProjectsSection.module.css';
import shared from './account.module.css';
import { projectsWord } from './accountWords';

/**
 * The workspace's projects in the same picker as connecting: tick what
 * Hoista watches, untick to take it out, rename the app. Saving changes only
 * Hoista; nothing changes at the provider.
 */
/** The repositories a code account's token reaches, as GitHub lists them. */
export function RepositoriesSection({ account }: { account: Account }) {
  return <RepositoryList accountId={account.id} />;
}

export function ProjectsSection({ account }: { account: Account }) {
  const projects = useProviderProjects(account.service);
  const empty = useEmptyProviderProjects(account.id);
  const list = useMemo(() => projects.data ?? [], [projects.data]);
  const picker = usePicker(list, 'in-projects', account.id);
  const { apply, busy, error } = useApplyPicker();
  const plan = picker.plan;
  const changes = plan.create.length + plan.add.length + plan.remove.length;

  async function save(): Promise<void> {
    if (await apply(plan)) showToast('Saved. Nothing changed on the provider.');
  }

  if (projects.isPending || empty.isPending) return <SkeletonRows count={4} />;
  const emptyProjects = empty.data ?? [];
  return (
    <>
      <ErrorMessage error={projects.error ?? empty.error} />
      {list.length === 0 && emptyProjects.length === 0 ? (
        <SettingGroup title={projectsWord(account.service)}>
          <SettingRow label={`Nothing found on ${serviceName(account.service)} yet`} />
        </SettingGroup>
      ) : (
        <ProjectPicker picker={picker} provider={account.service} empty={emptyProjects} />
      )}
      <ErrorMessage error={error} />
      <div className={styles.saveBar}>
        <span className={shared.small}>
          {changes === 0
            ? 'No changes'
            : `${picker.summary} · nothing changes at ${serviceName(account.service)}`}
        </span>
        <Button
          variant="primary"
          busy={busy}
          busyLabel="Saving"
          disabled={changes === 0}
          onClick={() => {
            void save();
          }}
        >
          Save changes
        </Button>
      </div>
    </>
  );
}
