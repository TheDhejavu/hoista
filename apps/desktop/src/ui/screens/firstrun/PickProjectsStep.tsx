/**
 * Setup step 2: everything found on the connected providers, in the same
 * import table as connecting a provider later, every account at once. Each
 * provider project comes in as a Hoista project of its name.
 */
import { useMemo } from 'react';
import { useNavigate } from 'react-router-dom';
import { entryOf, serviceName } from '@shared/providers';
import { useAccounts } from '@ui/api/accounts';
import { Button } from '@ui/components/Button';
import { Mascot } from '@ui/components/Mascot';
import { listOf, plural } from '@ui/lib/format';
import { ProjectPicker } from '@ui/screens/projects/ProjectPicker';
import { useApplyPicker, useFoundSnapshot, usePicker } from '@ui/screens/projects/usePicker';
import { SetupStep } from './SetupStep';
import { stepPath, usePicked, useSetup } from './useSetup';
import styles from './Setup.module.css';

export function PickProjectsStep() {
  const navigate = useNavigate();
  const picked = usePicked();
  const setup = useSetup();
  const accounts = useAccounts();
  const connected = new Set<string>((accounts.data?.items ?? []).map((item) => item.service));
  // Providers with projects to import: the connected ones, leaving out code (GitHub).
  const providers = picked.filter(
    (id) => connected.has(id) && entryOf(id).catalogue.category !== 'code',
  );
  const found = useFoundSnapshot(providers, accounts.isSuccess);
  const projects = useMemo(() => found.projects ?? [], [found.projects]);
  const picker = usePicker(projects, 'everything');
  const { apply, busy, error } = useApplyPicker();
  const looking = found.projects === null && !found.error;
  const created = picker.plan.create.length;
  const [first] = providers;

  async function next(): Promise<void> {
    if (picker.changed && !(await apply(picker.plan))) return;
    setup.setImported(created);
    void navigate(stepPath('ai', picked));
  }

  return (
    <SetupStep
      number={2}
      title="Pick what Hoista watches"
      lede="Each provider project comes in as a Hoista project of the same name. Untick what to leave out, or change where it goes. Nothing changes on your providers."
      back={
        <Button
          size="xl"
          onClick={() => {
            void navigate(stepPath('connect', picked));
          }}
        >
          Back
        </Button>
      }
      note={!looking && picker.summary}
      error={error ?? found.error}
      actions={
        <Button
          variant="accent"
          size="xl"
          busy={busy}
          busyLabel="Importing"
          disabled={looking}
          onClick={() => {
            void next();
          }}
        >
          {created > 0
            ? `Import ${plural(created, 'project')}`
            : picker.changed
              ? 'Import'
              : 'Continue'}
        </Button>
      }
    >
      {looking ? (
        <p className={styles.looking} role="status">
          <span className={styles.spinner} aria-hidden="true" />
          Looking for your projects on {listOf(providers.map((id) => serviceName(id)))}
        </p>
      ) : projects.length === 0 || !first ? (
        <div className={styles.nothing} role="status">
          <Mascot mood="empty" size={56} />
          <h2>Nothing to import yet</h2>
          <p>
            Hoista didn’t find any projects on{' '}
            {providers.length > 0 ? listOf(providers.map((id) => serviceName(id))) : 'them'}. When
            you or your agent make one, it shows up on Projects under Found on your providers.
          </p>
        </div>
      ) : (
        <div className={styles.picker}>
          <ProjectPicker picker={picker} provider={first} everyAccount />
        </div>
      )}
    </SetupStep>
  );
}
