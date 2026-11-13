/**
 * Project settings as cards. A managed project has its name and deleting it; a
 * watched project has more (see `WatchedSettings`), because its provider owns how it
 * runs and Hoista only keeps its own view of it.
 */
import { useId, useState, type ReactNode } from 'react';
import { useNavigate } from 'react-router-dom';
import type { ProjectDetail } from '@shared/api';
import { serviceName } from '@shared/providers';
import { useProject, useRemoveProject } from '@ui/api/projects';
import { useCan } from '@ui/api/providers';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { QueryState } from '@ui/components/QueryState';
import { SettingGroupSkeleton } from '@ui/components/SettingRow';
import { cx } from '@ui/lib/cx';
import { listOf } from '@ui/lib/format';
import { useRequiredParam } from '@ui/lib/useRequiredParam';
import { ProjectTab } from '../ProjectTab';
import styles from './SettingsTab.module.css';
import { CodeCard, NameCard, WatchedSettings } from './WatchedSettings';
import { EnvironmentCard } from './EnvironmentCard';

export function SettingsTab() {
  const projectId = useRequiredParam('projectId');
  const project = useProject(projectId);
  const can = useCan();
  // A watched project's provider owns how it runs; only Hoista's own view of it is set here.
  if (project.data?.project.mode === 'watching') {
    const deletable =
      project.data.resources.length > 0 &&
      project.data.resources.every((resource) => can(resource.provider, 'delete'));
    return (
      <ProjectTab>
        <div className={styles.column}>
          <WatchedSettings detail={project.data} />
          {deletable && <DeleteCard detail={project.data} />}
        </div>
      </ProjectTab>
    );
  }
  return <ManagedSettings projectId={projectId} />;
}

function ManagedSettings({ projectId }: { projectId: string }) {
  const project = useProject(projectId);

  return (
    <ProjectTab>
      <div className={styles.column}>
        <QueryState query={project} skeleton={<SettingGroupSkeleton cards={3} rows={1} />}>
          {(detail) => (
            <>
              <CodeCard
                detail={detail}
                provider={providersOf(detail)}
                // Hoista builds it, so nothing is connected at the provider.
                services={[]}
                managed
              />
              <NameCard detail={detail} provider={providersOf(detail)} />
              <EnvironmentCard detail={detail} />
              <DeleteCard detail={detail} />
            </>
          )}
        </QueryState>
      </div>
    </ProjectTab>
  );
}

function providersOf(detail: ProjectDetail): string {
  return (
    listOf([...new Set(detail.resources.map((resource) => serviceName(resource.provider)))]) ||
    'Its providers'
  );
}

export function Card({
  title,
  text,
  footer,
  danger = false,
  children,
}: {
  title: string;
  text: string;
  footer: ReactNode;
  danger?: boolean;
  children?: ReactNode;
}) {
  return (
    <section className={cx(styles.card, danger && styles.danger)}>
      <div className={styles.head}>
        <h2>{title}</h2>
        <p>{text}</p>
      </div>
      {children && <div className={styles.row}>{children}</div>}
      <div className={styles.foot}>{footer}</div>
    </section>
  );
}

function DeleteCard({ detail }: { detail: ProjectDetail }) {
  const [open, setOpen] = useState(false);
  const { project, resources } = detail;
  const providers = listOf([
    ...new Set(resources.map((resource) => serviceName(resource.provider))),
  ]);
  const hasData = resources.some(
    (resource) => resource.kind === 'database' || resource.kind === 'bucket',
  );

  return (
    <>
      <Card
        danger
        title={`Delete ${project.name}`}
        text={
          resources.length > 0
            ? `Deletes it${hasData ? ' and its data' : ''} on ${providers}, and removes it from Hoista.${hasData ? ' A backup of the data is saved to this computer first.' : ''}`
            : 'Removes it from Hoista. Nothing runs on a provider yet.'
        }
        footer={
          <>
            <span>This can’t be undone.</span>
            <Button
              variant="danger"
              large
              onClick={() => {
                setOpen(true);
              }}
            >
              Delete {project.name}
            </Button>
          </>
        }
      />
      {open && (
        <DeleteDialog
          projectName={project.name}
          projectId={project.id}
          hasData={hasData}
          onClose={() => {
            setOpen(false);
          }}
        />
      )}
    </>
  );
}

function DeleteDialog({
  projectName,
  projectId,
  hasData,
  onClose,
}: {
  projectName: string;
  projectId: string;
  hasData: boolean;
  onClose: () => void;
}) {
  const inputId = useId();
  const [typed, setTyped] = useState('');
  const [backup, setBackup] = useState(true);
  const remove = useRemoveProject(projectId);
  const navigate = useNavigate();

  return (
    <Dialog
      title={`Delete ${projectName}?`}
      eyebrow="This can’t be undone"
      onClose={onClose}
      actions={
        <>
          <Button large onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="danger"
            large
            disabled={typed !== projectName}
            busy={remove.isPending}
            busyLabel="Deleting"
            onClick={() => {
              remove.mutate(
                { backup: hasData && backup },
                {
                  onSuccess: () => {
                    void navigate('/');
                  },
                },
              );
            }}
          >
            Delete {projectName}
          </Button>
        </>
      }
    >
      {hasData && (
        <label className={styles.check}>
          <input
            type="checkbox"
            checked={backup}
            onChange={(event) => {
              setBackup(event.target.checked);
            }}
          />
          Save a backup of the data to this computer first
        </label>
      )}
      <label htmlFor={inputId} className={styles.label}>
        Type <span className="mono">{projectName}</span> to confirm
      </label>
      <input
        id={inputId}
        className={cx('mono', styles.input)}
        autoComplete="off"
        spellCheck={false}
        value={typed}
        onChange={(event) => {
          setTyped(event.target.value);
        }}
      />
      {remove.error && <ErrorMessage error={remove.error} />}
    </Dialog>
  );
}
