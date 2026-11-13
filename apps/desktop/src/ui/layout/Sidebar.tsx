/**
 * The left-hand navigation, as in the desktop design: the Hoista mark, search,
 * the five sections, then your most recently deployed projects. At the bottom, one
 * profile button that opens a menu with Settings, help and Lock.
 */
import { Activity, KeyRound, LayoutGrid, MessageCircle, Receipt, Search } from 'lucide-react';
import type { ReactNode } from 'react';
import { Link, NavLink } from 'react-router-dom';
import { projectNote } from '@ui/lib/projectLabel';
import { useProjects } from '@ui/api/projects';
import { ProjectPreviewIcon } from '@ui/components/ProjectPreviewIcon';
import { Dot } from '@ui/components/Dot';
import { statusLook } from '@ui/lib/status';
import { LogoMark } from '@ui/components/icons';
import { cx } from '@ui/lib/cx';
import { usePalette } from '@ui/palette/usePalette';
import { EnvironmentSwitcher } from './EnvironmentSwitcher';
import { ProfileMenuButton } from './ProfileMenu';
import { shortcutLabel } from '@ui/lib/keys';
import styles from './Sidebar.module.css';

const ICON_SIZE = 16;
/** How many projects the sidebar lists; the rest are one click away on Projects. */
const RECENT_PROJECTS = 10;

export function Sidebar() {
  const projects = useProjects({ sort: 'last_deploy' });
  const palette = usePalette();
  const projectCount = projects.data?.totals.projects;
  const recent = projects.data?.items.slice(0, RECENT_PROJECTS) ?? [];

  return (
    <aside className={styles.sidebar}>
      <div className={styles.windowButtonsSpace} />

      <Link to="/" className={styles.brand}>
        <span className={styles.mark}>
          <LogoMark size={15} />
        </span>
        Hoista
      </Link>

      <button type="button" className={styles.search} onClick={palette.open}>
        <Search size={14} />
        <span className={styles.searchLabel}>Search or jump to</span>
        <kbd className={styles.hint}>{shortcutLabel('k')}</kbd>
      </button>

      <EnvironmentSwitcher />

      {/* Only the lists scroll, so the profile menu can open above the footer unclipped. */}
      <div className={styles.groups}>
        <nav aria-label="Sections" className={styles.group}>
          <NavItem
            to="/"
            end
            icon={<LayoutGrid size={ICON_SIZE} />}
            label="Projects"
            count={projectCount}
          />
          <NavItem to="/deploys" icon={<Activity size={ICON_SIZE} />} label="Deploys" />
          <NavItem to="/agents" icon={<MessageCircle size={ICON_SIZE} />} label="Agents" />
          <NavItem to="/accounts" icon={<KeyRound size={ICON_SIZE} />} label="Accounts" />
          <NavItem to="/billing" icon={<Receipt size={ICON_SIZE} />} label="Billing" />
        </nav>

        {recent.length > 0 && (
          <nav aria-label="Recent projects" className={styles.group}>
            <h2 className={styles.groupTitle}>
              Recent projects
              {projectCount !== undefined && (
                <Link to="/" className={styles.seeAll}>
                  All {projectCount}
                </Link>
              )}
            </h2>
            {recent.map((project) => (
              <NavItem
                key={project.id}
                to={`/projects/${project.id}`}
                icon={<ProjectPreviewIcon project={project} size={14} />}
                label={project.name}
                note={projectNote(project, projects.data?.items ?? [])}
                trailing={<Dot tone={statusLook(project.status, project.idle_days).tone} small />}
                isProject
              />
            ))}
          </nav>
        )}
      </div>

      <div className={styles.footer}>
        <ProfileMenuButton />
      </div>
    </aside>
  );
}

function NavItem({
  to,
  end = false,
  icon,
  label,
  note,
  count,
  badge,
  trailing,
  isProject = false,
}: {
  to: string;
  end?: boolean;
  icon: ReactNode;
  label: string;
  /** Quieter words after the label, such as a project's providers. */
  note?: string | undefined;
  /** A quiet number on the right, such as how many projects there are. */
  count?: number | undefined;
  /** A highlighted number on the right, for things waiting on the user. */
  badge?: number | undefined;
  trailing?: ReactNode;
  isProject?: boolean;
}) {
  return (
    <NavLink
      to={to}
      end={end}
      className={({ isActive }) =>
        cx(styles.item, isProject && styles.project, isActive && styles.active)
      }
    >
      <span className={styles.icon}>{icon}</span>
      <span className={styles.label}>
        {label}
        {note && <span className={styles.note}> · {note}</span>}
      </span>
      {badge !== undefined && (
        <span className={styles.badge} aria-label={`${badge} waiting`}>
          {badge}
        </span>
      )}
      {count !== undefined && <span className={styles.count}>{count}</span>}
      {trailing}
    </NavLink>
  );
}
