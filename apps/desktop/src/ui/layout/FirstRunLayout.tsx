/**
 * The window during first run: no sidebar and no title bar, just the welcome
 * screen or the setup step being shown. Their empty areas also drag the window.
 */
import { Outlet } from 'react-router-dom';
import styles from './SidebarLayout.module.css';

export function FirstRunLayout() {
  return (
    <div className={styles.column}>
      <div className={styles.dragStrip} />
      <main className={styles.main}>
        <Outlet />
      </main>
    </div>
  );
}
