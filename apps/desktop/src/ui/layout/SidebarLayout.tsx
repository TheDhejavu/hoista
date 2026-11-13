/**
 * The window for every screen after first run: the sidebar on the left (with
 * search and the profile menu), and the current screen on the right under a
 * thin strip that drags the window. Sends people without a
 * profile to the welcome screen.
 */
import { Navigate, Outlet, useLocation } from 'react-router-dom';
import { useProfile } from '@ui/api/accounts';
import { ServerBar } from './ServerBar';
import { Sidebar } from './Sidebar';
import styles from './SidebarLayout.module.css';

export function SidebarLayout() {
  const profile = useProfile();
  const { pathname } = useLocation();

  if (profile.isSuccess && profile.data === null) {
    return <Navigate to="/welcome" replace />;
  }

  return (
    <>
      <Sidebar />
      <div className={styles.column}>
        <div className={styles.dragStrip} />
        <ServerBar />
        <main className={styles.main}>
          {/* A new key per screen replays the entrance; dialogs and tabs keep their screen. */}
          <div key={screenKey(pathname)} className={styles.screen}>
            <Outlet />
          </div>
        </main>
      </div>
    </>
  );
}

/**
 * Which screen a path belongs to. `/projects/moodboard/logs` and
 * `/accounts/connect/neon` stay on the same screen (a project's tab, a dialog
 * over Accounts), so moving between them doesn't replay the entrance.
 */
function screenKey(pathname: string): string {
  const [section = '', id] = pathname.split('/').filter(Boolean);
  return ['projects', 'deploys'].includes(section) && id ? `${section}/${id}` : section;
}
