/**
 * The Projects table: one row per project with the columns picked in the Columns
 * dropdown. A row opens its project; the ··· menu holds the quick actions.
 */
import { MoreVertical } from 'lucide-react';
import { useEffect, useState, type ReactNode } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import type { ProjectListItem } from '@shared/api';
import { serviceName } from '@shared/providers';
import { openExternal } from '@ui/api/system';
import { ActorIcon, AgentIcon, PersonAvatar } from '@ui/components/AgentIcon';
import { ProjectPreviewIcon } from '@ui/components/ProjectPreviewIcon';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { BarSparkline } from '@ui/components/Sparkline';
import { statusLook } from '@ui/lib/status';
import { useCopy } from '@ui/lib/useCopy';
import { cx } from '@ui/lib/cx';
import { formatCount, hostOf, shortAgo } from '@ui/lib/format';
import { usePopover } from '@ui/lib/usePopover';
import { COLUMNS, type ColumnId } from './columns';
import styles from './ProjectsTable.module.css';

export function ProjectsTable({
  projects,
  columns,
}: {
  projects: ProjectListItem[];
  columns: ColumnId[];
}) {
  const shown = COLUMNS.filter((column) => columns.includes(column.id));

  return (
    <div className={styles.frame}>
      <table className={styles.table}>
        <thead>
          <tr>
            <th scope="col" className={styles.iconCell}>
              <span className="visually-hidden">Preview</span>
            </th>
            <th scope="col" className={styles.project}>
              Project
            </th>
            {shown.map((column) => (
              <th key={column.id} scope="col" className={styles[column.id]}>
                {column.label}
              </th>
            ))}
            <th scope="col" className={styles.menuCell}>
              <span className="visually-hidden">Actions</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {projects.map((project) => (
            <ProjectRow
              key={project.id}
              project={project}
              columns={shown.map((column) => column.id)}
            />
          ))}
        </tbody>
      </table>
    </div>
  );
}

function ProjectRow({ project, columns }: { project: ProjectListItem; columns: ColumnId[] }) {
  const navigate = useNavigate();
  const path = `/projects/${project.id}`;

  return (
    <tr
      onClick={() => {
        void navigate(path);
      }}
    >
      <td className={styles.iconCell}>
        <ProjectPreviewIcon project={project} size={26} />
      </td>
      <td className={styles.project}>
        <Link to={path} className={styles.name}>
          {project.name}
        </Link>
        <span className={styles.address}>{project.url ? hostOf(project.url) : 'No domain'}</span>
      </td>
      {columns.map((id) => (
        <td key={id} className={styles[id]}>
          <Cell id={id} project={project} />
        </td>
      ))}
      <td
        className={styles.menuCell}
        onClick={(event) => {
          event.stopPropagation();
        }}
      >
        <RowMenu project={project} />
      </td>
    </tr>
  );
}

function Cell({ id, project }: { id: ColumnId; project: ProjectListItem }): ReactNode {
  switch (id) {
    case 'status':
      return <StatusPill project={project} />;
    case 'providers':
      return (
        <span className={styles.providerList}>
          {project.providers.map((provider) => (
            <span key={provider} className={styles.providerTile} title={serviceName(provider)}>
              <ServiceIcon id={provider} size={12} />
            </span>
          ))}
        </span>
      );
    case 'mode':
      return (
        <span className={styles.muted}>
          {project.mode === 'watching' ? 'Watching only' : 'Hoista'}
        </span>
      );
    case 'runs_on':
      return <span className={styles.muted}>{project.runs_on.join(' · ')}</span>;
    case 'requests':
      return (
        <span className={styles.reqWrap}>
          {project.sparkline && <BarSparkline values={project.sparkline} height={20} />}
          <span className={cx('mono', styles.count)}>
            {project.requests_7d === null ? '—' : formatCount(project.requests_7d)}
          </span>
        </span>
      );
    case 'last_deploy':
      return <LastDeploy project={project} />;
  }
}

function StatusPill({ project }: { project: ProjectListItem }) {
  const { label, tone } = statusLook(project.status, project.idle_days);
  return (
    <span className={cx(styles.pill, project.status === 'failing' && styles.pillBad)}>
      <span className={cx(styles.dot, styles[tone])} />
      {label}
    </span>
  );
}

function LastDeploy({ project }: { project: ProjectListItem }) {
  if (!project.last_deploy)
    return (
      <span className={styles.muted}>
        {project.ships_code === false ? 'Nothing to deploy' : 'Never'}
      </span>
    );
  const { by } = project.last_deploy;

  return (
    <span className={styles.deploy}>
      {by.kind === 'agent' ? (
        <AgentIcon name={by.name} small />
      ) : by.kind === 'system' ? (
        <ActorIcon actor={by} />
      ) : (
        <PersonAvatar />
      )}
      <span className={styles.muted}>{shortAgo(project.last_deploy.at)}</span>
    </span>
  );
}

function RowMenu({ project }: { project: ProjectListItem }) {
  const navigate = useNavigate();
  const { open, setOpen, ref } = usePopover<HTMLDivElement>();
  const { copied, copy } = useCopy();
  const { url } = project;
  // The table scrolls sideways, which would clip a menu inside it, so the
  // menu is placed against the window, under the button.
  const [place, setPlace] = useState<{ top: number; right: number } | null>(null);
  // A placed menu would drift from its button on scroll; close it instead.
  useEffect(() => {
    if (!open) return;
    const close = (): void => {
      setOpen(false);
    };
    window.addEventListener('scroll', close, true);
    window.addEventListener('resize', close);
    return () => {
      window.removeEventListener('scroll', close, true);
      window.removeEventListener('resize', close);
    };
  }, [open, setOpen]);

  return (
    <div ref={ref} className={styles.menuWrap}>
      <button
        type="button"
        className={styles.more}
        aria-label={`More for ${project.name}`}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={(event) => {
          const box = event.currentTarget.getBoundingClientRect();
          setPlace({ top: box.bottom + 4, right: window.innerWidth - box.right });
          setOpen(!open);
        }}
      >
        <MoreVertical size={16} aria-hidden="true" />
      </button>
      {open && (
        <div
          className={styles.menu}
          role="menu"
          style={place ? { position: 'fixed', top: place.top, right: place.right } : undefined}
        >
          <MenuItem
            onClick={() => {
              void navigate(`/projects/${project.id}`);
            }}
          >
            Open project
          </MenuItem>
          {url && (
            <MenuItem
              onClick={() => {
                void openExternal(url);
                setOpen(false);
              }}
            >
              Open site ↗
            </MenuItem>
          )}
          {url && (
            <MenuItem
              onClick={() => {
                copy(url);
              }}
            >
              {copied ? 'Address copied' : 'Copy address'}
            </MenuItem>
          )}
          <MenuItem
            onClick={() => {
              void navigate(`/projects/${project.id}/settings`);
            }}
          >
            Settings
          </MenuItem>
        </div>
      )}
    </div>
  );
}

function MenuItem({ onClick, children }: { onClick: () => void; children: ReactNode }) {
  return (
    <button type="button" role="menuitem" className={styles.menuItem} onClick={onClick}>
      {children}
    </button>
  );
}
