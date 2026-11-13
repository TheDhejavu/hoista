/**
 * The window's title follows the screen ("moodboard · Logs"), so the Window
 * menu, ⌘` and Mission Control say where each window is, as in any desktop app.
 */
import { useEffect } from 'react';
import { useLocation } from 'react-router-dom';
import { useProjectNames } from '@ui/lib/useProjectNames';

const SECTIONS: Record<string, string> = {
  '': 'Projects',
  deploy: 'Deploys',
  deploys: 'Deploy',
  agents: 'Agents',
  accounts: 'Accounts',
  billing: 'Billing',
  setup: 'Set up Hoista',
  welcome: 'Welcome',
};

const TABS: Record<string, string> = {
  map: 'Map',
  deploys: 'Deploys',
  logs: 'Logs',
  metrics: 'Metrics',
  env: 'Variables',
  domains: 'Domains',
  settings: 'Settings',
  provider: 'Providers',
  activity: 'Activity',
  resources: 'Resource',
};

export function useWindowTitle(): void {
  const { pathname } = useLocation();
  const projectName = useProjectNames();

  useEffect(() => {
    const [section = '', id, tab] = pathname.split('/').filter(Boolean);
    let title: string;
    if (section === 'projects' && id) {
      const name = projectName(decodeURIComponent(id));
      title = tab && TABS[tab] ? `${name} · ${TABS[tab]}` : name;
    } else {
      title = SECTIONS[section] ?? '';
    }
    document.title = title ? `${title} — Hoista` : 'Hoista';
  }, [pathname, projectName]);
}
