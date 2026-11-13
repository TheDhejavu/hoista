/**
 * Needs attention (`GET /attention`) from the daemon's feed: pending
 * approvals, failed deploys and idle resources, plus
 * accounts whose token stopped working.
 *
 * Every item comes from an engine record; the adapter only words it. The
 * engine pages approvals and project notices separately; this returns the first
 * page of each, which is what the strip and the menu bar show.
 */
import type {
  Account as DaemonAccount,
  ProjectOverview,
  AttentionFeed,
  AttentionNotice,
  Deploy as DaemonDeploy,
  Operation,
} from '@engine-api';
import type { AttentionItem, Page } from '@shared/api';
import { SERVICES } from '@shared/providers';
import type { Router } from '@shared/router';
import { abilitiesOf, type Abilities } from './abilities';
import { allPages, orNull, type Call } from './call';

export function addAttentionRoutes(
  router: Router,
  call: Call,
  abilities: () => Promise<Abilities> = abilitiesOf(call),
): void {
  router.on('GET', '/attention', async (): Promise<Page<AttentionItem>> => {
    const [feed, projects, accounts, operations, known] = await Promise.all([
      call<AttentionFeed>('GET', '/attention'),
      allPages<ProjectOverview>(call, '/projects/summaries'),
      allPages<DaemonAccount>(call, '/accounts'),
      allPages<Operation>(call, '/operations'),
      abilities(),
    ]);
    const overviews = new Map(projects.map((overview) => [overview.project.id, overview]));
    const failed = await Promise.all(
      feed.notices
        .filter((notice) => notice.kind === 'deployment_failed')
        .map((notice) =>
          orNull(call<DaemonDeploy>('GET', `/deploys/${encodeURIComponent(notice.deploy_id)}`)),
        ),
    );
    const deploys = new Map(
      failed
        .filter((deploy): deploy is DaemonDeploy => deploy !== null)
        .map((deploy) => [deploy.id, deploy]),
    );
    return {
      items: [
        ...operations.flatMap(unclearItem),
        ...feed.notices.flatMap((notice) => noticeItem(notice, overviews, deploys)),
        ...accounts.flatMap((account) =>
          accountItem(account, known.can(account.provider, 'browser_sign_in')),
        ),
      ],
      cursor: null,
    };
  });
}

export function noticeItem(
  notice: AttentionNotice,
  overviews: ReadonlyMap<string, ProjectOverview>,
  deploys: ReadonlyMap<string, DaemonDeploy>,
): AttentionItem[] {
  const overview = overviews.get(notice.project_id);
  const project = overview?.project.name ?? 'A project';
  const base = { project_id: notice.project_id, approval_id: null, agent: null, waiting_on: null };
  switch (notice.kind) {
    case 'deployment_failed': {
      const failure = deploys.get(notice.deploy_id)?.failure;
      const missing = failure?.missing_secrets ?? [];
      return [
        {
          ...base,
          id: `failure:${notice.deploy_id}`,
          kind: 'failure',
          tone: 'bad',
          title: `${project} can’t deploy`,
          body:
            missing.length > 0
              ? `Missing ${missing.join(', ')}`
              : (failure?.message ?? 'Open the project to see what went wrong.'),
        },
      ];
    }
    case 'resource_failing': {
      const resource = overview?.resources.find((candidate) => candidate.id === notice.resource_id);
      const provider = resource ? SERVICES[resource.provider].name : 'its provider';
      return [
        {
          ...base,
          id: `failing:${notice.resource_id}`,
          kind: 'failure',
          tone: 'bad',
          title: `${resource?.name ?? project} is failing on ${provider}`,
          body: `${provider} reports its latest deployment crashed or failed. Open its logs to see why.`,
        },
      ];
    }
    case 'idle_resource': {
      const resource = overview?.resources.find((candidate) => candidate.id === notice.resource_id);
      return [
        {
          ...base,
          id: `idle:${notice.resource_id}`,
          kind: 'saving',
          tone: 'warn',
          title: `${resource?.name ?? project} had no requests for ${String(notice.days)} days`,
          body: 'Pause or remove it if nobody uses it.',
        },
      ];
    }
  }
}

/** A change Hoista didn't hear back about: the person checks the provider and answers. */
export function unclearItem(operation: Operation): AttentionItem[] {
  if (operation.state !== 'unclear') return [];
  const name = SERVICES[operation.provider].name;
  return [
    {
      id: `unclear:${operation.id}`,
      kind: 'unclear',
      tone: 'warn',
      project_id: null,
      title: `Did this happen at ${name}?`,
      body: `${capitalize(operation.target)} · Hoista didn't hear back`,
      approval_id: null,
      agent: null,
      waiting_on: null,
      operation: {
        id: operation.id,
        service: operation.provider,
        target: operation.target,
        started_at: operation.started_at,
      },
    },
  ];
}

function capitalize(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** `signsIn`: the provider reconnects by signing in on its page, not with a new token. */
function accountItem(account: DaemonAccount, signsIn: boolean): AttentionItem[] {
  if (account.status === 'connected') return [];
  const name = SERVICES[account.provider].name;
  return [
    {
      id: `account:${account.id}`,
      kind: 'account',
      tone: 'bad',
      project_id: null,
      title:
        account.status === 'missing_scopes'
          ? `${name} token is missing permissions`
          : account.status === 'expired'
            ? signsIn
              ? `${name} needs reconnecting`
              : `${name} needs a new token`
            : `${name} is disconnected`,
      body: `${account.label} · projects keep running at ${name}.`,
      approval_id: null,
      agent: null,
      waiting_on: null,
    },
  ];
}
