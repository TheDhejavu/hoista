/**
 * The few moments worth interrupting the user for, read from the engine.
 *
 * An agent asking for approval, or waiting for a provider to be reconnected,
 * always notifies, because the agent is stuck until the user answers. Deploy
 * results and spend alerts follow Settings · General · Notify me. How a
 * notice is shown is up to the `Notifier` passed in.
 */
import type { ProjectDetail, Deploy, EngineEvent } from '@shared/api';
import type { Engine } from '@shared/engine';
import { reconnectPath } from '@shared/reconnect';
import { get, readStatus, type Status } from './status';

/** Shows a notice. Skips it while the main window is in front, since the app already shows it. */
export interface Notifier {
  show(note: Note): void;
}

export interface Note {
  /** Replaces an earlier notification with the same key instead of stacking. */
  key: string;
  title: string;
  body: string;
  /** Where clicking it goes. */
  target: string;
}

export function watchForNotices(engine: Engine, notifier: Notifier): void {
  let status: Status | null = null;
  /** Reconnect requests already notified, so each one notifies once. */
  const seenWaiting = new Set<string>();
  let primed = false;

  function show(note: Note): void {
    notifier.show(note);
  }

  async function projectName(projectId: string): Promise<string> {
    const detail = await get<ProjectDetail>(engine, `/projects/${projectId}`);
    return detail?.project.name ?? 'Your project';
  }

  /** Notifies for new reconnect requests. The first read only records what is already there. */
  async function checkWaiting(): Promise<void> {
    status = await readStatus(engine);
    for (const item of status.waiting) {
      if (seenWaiting.has(item.id) || !item.waiting_on) continue;
      seenWaiting.add(item.id);
      if (!primed) continue;
      const waiting = item.waiting_on;
      show({
        key: `waiting:${item.id}`,
        title: `${waiting.agent} is waiting for you`,
        body: item.title,
        target: reconnectPath(waiting),
      });
    }
    // An agent asking to connect waits for the person; say so once.
    for (const request of status.requests) {
      if (seenWaiting.has(request.id)) continue;
      seenWaiting.add(request.id);
      if (!primed) continue;
      show({
        key: `request:${request.id}`,
        title: `${request.claimed_name} wants to connect`,
        body: `${request.answer_in === 'browser' ? 'Approve it on Hoista’s page in your browser' : 'Review it in Hoista'}${request.code ? ` · code ${request.code}` : ''}`,
        target: '/agents',
      });
    }
    primed = true;
  }

  async function onEvent(event: EngineEvent): Promise<void> {
    const notify = status?.notify ?? 'problems';
    const subject = event.subject_id;
    switch (event.kind) {
      // Approvals are asked and answered in the agent itself, not here.
      case 'deploy.finished': {
        if (notify === 'never' || !subject || !event.project_id) break;
        const deploy = await get<Deploy>(engine, `/deploys/${subject}`);
        if (!deploy) break;
        const failed = deploy.status === 'failed';
        if (!failed && (notify !== 'deploys' || deploy.status !== 'live')) break;
        const name = await projectName(event.project_id);
        show({
          key: `deploy:${event.project_id}`,
          title: failed ? `${name} didn't go live` : `${name} is live`,
          body: failed
            ? (deploy.failure?.message ?? 'Open Hoista to see what went wrong.')
            : (deploy.url ?? 'Your new version is serving traffic.'),
          target: `/projects/${event.project_id}`,
        });
        break;
      }
      case 'resource.observed': {
        // The provider reports a watched resource crashed or failed.
        if (notify === 'never' || !event.project_id || !event.summary.endsWith(' is failing'))
          break;
        const name = await projectName(event.project_id);
        show({
          key: `failing:${event.project_id}`,
          title: `${name} needs a look`,
          body: `${event.summary.replace(/ is failing$/, '')} is failing on its provider. Open its logs to see why.`,
          target: `/projects/${event.project_id}`,
        });
        break;
      }
      default:
        break;
    }
    if (event.kind.startsWith('approval.') || event.kind.startsWith('account.')) {
      await checkWaiting();
    }
  }

  engine.subscribe((event) => void onEvent(event));
  void checkWaiting();
}
