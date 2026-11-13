import { describe, expect, it } from 'vitest';
import type { ProjectSummary } from '@shared/api';
import { buildCommands, filterCommands, splitMatch } from './commands';

const project = (id: string): ProjectSummary => ({
  id,
  name: id,
  url: `https://${id}.example.com`,
  status: 'live',
  idle_days: null,
  mode: 'managed',
  environment_id: 'env_default',
  imported: false,
  runs_on: [],
  providers: [],
  requests_7d: null,
  sparkline: null,
  last_deploy: null,
});

const projects = [project('moodboard'), project('invoice-bot')];

const moodboard = projects.filter((project) => project.id === 'moodboard');

describe('palette commands', () => {
  it('finds commands by any part of their label', () => {
    const labels = filterCommands(buildCommands(moodboard), 'logs').map((command) => command.label);
    expect(labels).toEqual(['moodboard · Logs']);
  });

  it('sends deploying to the agent guide, never a deploy form', () => {
    const [deploy] = filterCommands(buildCommands(moodboard), 'deploy');
    expect(deploy).toMatchObject({ to: '/deploys', label: 'Deploy with your agent' });
  });

  it('opens Settings as a dialog rather than a screen', () => {
    const [settings] = filterCommands(buildCommands([]), 'settings');
    expect(settings).toMatchObject({ opens: 'settings' });
  });

  it('answers to the words people use, not only the labels', () => {
    const labels = filterCommands(buildCommands(moodboard), 'redeploy').map((c) => c.label);
    expect(labels).toContain('moodboard · Deploys');
    const env = filterCommands(buildCommands(moodboard), 'env').map((c) => c.label);
    expect(env).toContain('moodboard · Variables');
  });

  it('puts the open project first', () => {
    const [first] = buildCommands(projects, 'moodboard');
    expect(first).toMatchObject({ label: 'moodboard', group: 'This project' });
  });

  it('offers the open project’s CLI commands and id to copy', () => {
    const copies = buildCommands(projects, 'moodboard').filter((c) => 'copy' in c);
    expect(copies.map((c) => ('copy' in c ? c.copy : ''))).toEqual(
      expect.arrayContaining([
        'hoist logs moodboard --follow',
        'hoist status moodboard',
        'moodboard',
      ]),
    );
    // Only for the project that's open, not every project.
    expect(buildCommands(projects).some((c) => c.id.startsWith('cli-'))).toBe(false);
  });

  it('shows the shortcut beside a page that has one', () => {
    const [deploys] = filterCommands(buildCommands([]), 'Deploys');
    expect(deploys).toMatchObject({ to: '/deploys', shortcut: '2' });
  });

  it('splits a label around the match', () => {
    expect(splitMatch('Redeploy moodboard', 'RED')).toEqual(['', 'Red', 'eploy moodboard']);
    expect(splitMatch('Agents', 'xyz')).toEqual(['Agents', '', '']);
  });
});
