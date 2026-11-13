/**
 * The Agents screen from the daemon's agents and activity.
 *
 * An agent's access comes from its scopes (`apps:read` look, `apps:write`
 * deploy, `apps:create` create) and its grant: the projects and environments it
 * may touch, tools it may never use and when its access ends. The engine
 * enforces both. It keeps no spending cap or webhook, so those are null and
 * changing them is refused with a plain message instead of pretending to save. Activity is the
 * engine's `agent.tool` audit trail for that agent.
 */
import type {
  ActivityPage,
  Agent as DaemonAgent,
  Approval as DaemonApproval,
  EnrollmentRequest,
  EventDto,
  Page as DaemonPage,
} from '@engine-api';
import type {
  AccessLevel,
  Agent,
  AgentAddress,
  AgentLink,
  AgentActivity,
  AgentCall,
  AgentUpdate,
  ConnectionRequest,
  Decision,
  Page,
} from '@shared/api';
import { agentDisplayName, isCodingTool } from '@shared/agentLogos';
import { RouteError, type Router } from '@shared/router';
import { allPages, query, type Call } from './call';
import { MINUTE_MS } from '@shared/time';

const SCOPES: Record<AccessLevel, string[]> = {
  look: ['apps:read'],
  ask: ['apps:read', 'apps:write', 'apps:create'],
  full: ['apps:read', 'apps:write', 'apps:create'],
};
const STRICTER_RULE = { ask: 'always_ask', look: 'view_only' } as const;
const RESTRICTION = { ask: 'always_ask', look: 'view_only', hidden: 'no_access' } as const;
const RESTRICTION_OF = { always_ask: 'ask', view_only: 'look', no_access: 'hidden' } as const;
/** Seen within this long counts as active. */
const ACTIVE_MS = 15 * MINUTE_MS;

/** View only without write access; otherwise Full access or Ask for approval. */
function accessOf(scopes: string[], withoutAsking = false): AccessLevel {
  if (!scopes.includes('apps:write')) return 'look';
  return withoutAsking ? 'full' : 'ask';
}

function toAgent(agent: DaemonAgent, waiting: boolean, now = Date.now()): Agent {
  const seen = Date.parse(agent.last_seen);
  return {
    id: agent.id,
    name: agentDisplayName(agent.name),
    kind: agent.transport === 'stdio' || isCodingTool(agent.name) ? 'coding_tool' : 'chat_app',
    transport: agent.transport,
    access: accessOf(agent.scopes, agent.grant.without_asking),
    environment_rules: Object.fromEntries(
      Object.entries(agent.grant.environment_rules).map(([id, rule]) => [
        id,
        rule === 'view_only' ? ('look' as const) : ('ask' as const),
      ]),
    ),
    exceptions: agent.grant.exceptions.map((exception) => ({
      group: exception.group ?? null,
      project_id: exception.project_id ?? null,
      resource_id: exception.resource_id ?? null,
      access: RESTRICTION_OF[exception.access],
    })),
    projects: agent.grant.projects ?? null,
    environment_ids: agent.grant.environment_ids ?? null,
    never: agent.grant.deny,
    access_ends_at: agent.grant.expires_at ?? null,
    webhook: null,
    last_seen: Number.isNaN(seen) ? null : agent.last_seen,
    status: waiting ? 'waiting' : !Number.isNaN(seen) && now - seen < ACTIVE_MS ? 'active' : 'idle',
  };
}

/** Whether a host is this computer, as a tool's sign-in callback names it. */
function isLoopback(host: string | undefined): boolean {
  return host === '127.0.0.1' || host === 'localhost' || host === '[::1]' || host === '::1';
}

/** Web agents' requests are named after their sign-in request: `oauth-<id>`. */
const WEB = 'oauth-';

/** Tools that only read, which the Agents page groups together. */
export function isReadOnly(tool: string): boolean {
  return (
    /^(get|list|suggest|resolve)_/.test(tool) ||
    tool === 'describe_provider_operation' ||
    tool === 'wait_for_deploy'
  );
}

const SAID: Record<string, string> = {
  list_projects: 'Listed projects',
  get_project: 'Read a project',
  get_project_map: 'Read the project map',
  get_attention: 'Checked what needs you',
  get_logs: 'Read logs',
  get_log_history: 'Read older logs',
  get_bills: 'Read the bills',
  get_usage: 'Read usage',
  get_provider_details: 'Read provider details',
  get_resource: 'Read a resource',
  set_secrets: 'Set variables',
  delete_shared_variable: 'Removed a shared variable',
  get_metrics: 'Read metrics',
  get_deploy: 'Read a deploy',
  get_approval: 'Checked an approval',
  list_provider_operations: 'Listed provider operations',
  describe_provider_operation: 'Read a provider operation',
  run_provider_operation: 'Ran a provider operation',
  deploy: 'Deployed',
  deploy_folder: 'Deployed a folder',
  deploy_commit: 'Deployed a commit',
  deploy_latest_commit: 'Deployed the latest commit',
  plan_deploy: 'Planned a deploy',
  cancel_deploy: 'Cancelled a deploy',
  rollback: 'Rolled back',
  rollback_resource: 'Rolled back a resource',
  redeploy_resource: 'Redeployed',
  restart_resource: 'Restarted',
  stop_resource: 'Stopped the running deployment',
  remove_service: 'Removed a service',
  create_service: 'Created a service',
  add_database: 'Added a database',
  create_bucket: 'Created a bucket',
  set_secret: 'Set a variable',
  delete_secret: 'Removed a variable',
  replace_environment: 'Replaced variables',
  add_domain: 'Added a domain',
  add_resource_domain: 'Added a domain',
  remove_resource_domain: 'Removed a domain',
  update_resource_domain: 'Changed a domain',
  delete_domain: 'Removed a domain',
  check_domain: 'Checked a domain',
  list_domains: 'Listed domains',
  retry_domain_certificate: 'Retried a certificate',
  add_volume: 'Added a volume',
  remove_volume: 'Removed a volume',
  add_tcp_proxy: 'Added a TCP proxy',
  remove_tcp_proxy: 'Removed a TCP proxy',
  rename_private_host: 'Renamed a private address',
  configure_resource: 'Changed settings',
  connect_repo: 'Connected a repo',
  disconnect_source: 'Disconnected the source',
  create_project: 'Created a project',
  create_project_at_provider: 'Created a project on its provider',
  wait_for_deploy: 'Followed a deploy',
  delete_project: 'Deleted a project',
  delete_resource: 'Deleted a resource',
  connect_account: 'Connected an account',
  get_my_access: 'Checked its access',
  list_environments: 'Listed environments',
  move_project: 'Moved a project',
  get_database_operations: 'Read database operations',
  refresh_resource_bindings: 'Refreshed connections',
  give_key: 'Gave a key',
  move_resources: 'Moved resources',
  get_plan: 'Read a deploy plan',
  edit_plan: 'Changed a deploy plan',
  list_repositories: 'Listed repositories',
  link_repository: 'Linked a repository',
  link_folder: 'Linked a folder',
  get_database: 'Read a database',
  get_deployment_request: 'Read a deploy request',
  rename_resource_domain: 'Renamed a domain',
  add_to_project: 'Added to a project',
  list_import_candidates: 'Listed what can be brought in',
  set_resource_ignored: 'Changed what Hoista watches',
  suggest_imports: 'Suggested what to bring in',
  import_resources: 'Brought in resources',
};

/** The tool an `agent.tool` summary names (`deploy: completed`). */
export function calledTool(summary: string): { tool: string; result: string } {
  const [tool = '', result = ''] = summary.split(/:\s*/, 2);
  return { tool, result };
}

/** `remove_resource_domain` → `Removed a domain`; unknown tools read `Used set_x`. */
export function toolSentence(tool: string): string {
  return SAID[tool] ?? `Used ${tool}`;
}

function toAgentActivity(event: EventDto, names: Map<string, string>): AgentActivity {
  const called = calledTool(event.summary);
  const tool = called.tool || event.kind;
  const result = called.result;
  const failed = /rejected|failed|denied/.test(result);
  return {
    id: event.id,
    at: event.at,
    agent_id: event.actor,
    agent_name: names.get(event.actor) ?? 'A removed agent',
    summary: toolSentence(tool),
    project_id: event.project_id,
    tool,
    detail: null,
    tone: failed ? 'bad' : result.includes('pending') ? 'warn' : 'good',
    action: null,
    result: result ? `${result.charAt(0).toUpperCase()}${result.slice(1)}` : '',
    read_only: isReadOnly(tool),
    reason: null,
    sent: null,
    happened: [],
    approval_id: null,
    waiting_on: null,
  };
}

export interface CallDetail {
  tool?: string;
  sent?: unknown;
  result?: { outcome?: string; message?: string | null; approval_id?: string | null };
}

const OUTCOMES: Record<string, string> = {
  completed: 'Completed',
  rejected: 'Refused',
  pending: 'Waiting for your approval',
  pending_approval: 'Waiting for your approval',
  declined: 'You declined it',
};

/** `deploy { project: "moodboard" }`, laid out over lines like code. */
function callText(tool: string, args: unknown): string {
  const key = (name: string) => (/^[A-Za-z_$][\w$]*$/.test(name) ? name : JSON.stringify(name));
  const value = (item: unknown, indent: string): string => {
    if (Array.isArray(item)) {
      if (item.length === 0) return '[]';
      const inner = `${indent}  `;
      return `[\n${item.map((entry) => `${inner}${value(entry, inner)}`).join(',\n')}\n${indent}]`;
    }
    if (item && typeof item === 'object') {
      const entries = Object.entries(item);
      if (entries.length === 0) return '{}';
      const inner = `${indent}  `;
      return `{\n${entries
        .map(([name, entry]) => `${inner}${key(name)}: ${value(entry, inner)}`)
        .join('\n')}\n${indent}}`;
    }
    return item === undefined ? 'null' : JSON.stringify(item);
  };
  return `${tool} ${value(args, '')}`;
}

/** The engine's recorded detail for one call, as the Agents page shows it. */
export function toAgentCall(detail: CallDetail | null): AgentCall {
  if (!detail) return { sent: null, status: null, happened: [], approval_id: null };
  const outcome = detail.result?.outcome ?? 'completed';
  const message = detail.result?.message;
  return {
    sent: detail.sent === undefined ? null : callText(detail.tool ?? 'call', detail.sent),
    status: OUTCOMES[outcome] ?? `${outcome.charAt(0).toUpperCase()}${outcome.slice(1)}`,
    happened: message ? [message] : [],
    approval_id: detail.result?.approval_id ?? null,
  };
}

export function addAgentRoutes(router: Router, call: Call): void {
  async function agentsPage(): Promise<Page<Agent>> {
    const [agents, approvals] = await Promise.all([
      call<DaemonPage<DaemonAgent>>('GET', '/agents'),
      allPages<DaemonApproval>(call, '/approvals'),
    ]);
    const pending = approvals.filter((approval) => approval.decision === 'pending');
    const waiting = new Set(pending.map((approval) => approval.agent_id));
    // A web agent exists from the moment it asks to sign in. Until that is
    // allowed it's a request, not a connected agent; declined or expired, it
    // never connected.
    const notLetIn = new Set(
      approvals
        .filter(
          (approval) =>
            approval.action === 'connect_agent' &&
            approval.decision !== 'allowed' &&
            approval.decision !== 'consumed',
        )
        .map((approval) => approval.agent_id),
    );
    return {
      items: agents.items
        .filter((agent) => !agent.revoked && !notLetIn.has(agent.id))
        .map((agent) => toAgent(agent, waiting.has(agent.id))),
      cursor: agents.next_cursor,
    };
  }

  router.on('GET', '/agents', agentsPage);

  router.on('GET', '/agents/activity', async ({ query: search }): Promise<Page<AgentActivity>> => {
    const agentId = search.get('agent');
    const [page, agents] = await Promise.all([
      call<ActivityPage>('GET', `/activity${query({ actor: agentId, kind: 'agent.tool' })}`),
      call<DaemonPage<DaemonAgent>>('GET', '/agents'),
    ]);
    const names = new Map(agents.items.map((agent) => [agent.id, agentDisplayName(agent.name)]));
    return {
      // The engine pages agent calls on their own, so other activity can't
      // push them off the page. Calls by agents since removed stay listed.
      items: page.items
        .filter((event) => event.kind === 'agent.tool')
        .map((event) => toAgentActivity(event, names)),
      cursor: page.next_cursor,
    };
  });

  router.on('GET', '/agents/activity/:id', async ({ params }): Promise<AgentCall> => {
    const id = encodeURIComponent(params.id ?? '');
    const answer = await call<{ detail: CallDetail | null }>('GET', `/events/${id}/detail`);
    return toAgentCall(answer.detail);
  });

  router.on('PATCH', '/agents/:id', async ({ params, body }): Promise<Agent> => {
    const id = params.id ?? '';
    const update = (body ?? {}) as AgentUpdate;
    if (update.webhook !== undefined) {
      throw new RouteError(
        'agent.limits_unsupported',
        "Hoista doesn't keep a webhook for an agent yet.",
      );
    }
    if (
      update.projects !== undefined ||
      update.environment_ids !== undefined ||
      update.never !== undefined ||
      update.access_ends_at !== undefined ||
      update.access !== undefined ||
      update.environment_rules !== undefined ||
      update.exceptions !== undefined
    ) {
      // The grant is replaced as a whole, so unchanged resources are sent as they are.
      const current = (await agentsPage()).items.find((agent) => agent.id === id);
      await call('PUT', `/agents/${encodeURIComponent(id)}/grant`, {
        projects: update.projects !== undefined ? update.projects : (current?.projects ?? null),
        environment_ids:
          update.environment_ids !== undefined
            ? update.environment_ids
            : (current?.environment_ids ?? null),
        deny: update.never ?? current?.never ?? [],
        expires_at:
          update.access_ends_at !== undefined
            ? update.access_ends_at
            : (current?.access_ends_at ?? null),
        without_asking: (update.access ?? current?.access) === 'full',
        environment_rules: Object.fromEntries(
          Object.entries(update.environment_rules ?? current?.environment_rules ?? {}).map(
            ([environment, rule]) => [environment, STRICTER_RULE[rule]],
          ),
        ),
        exceptions: (update.exceptions ?? current?.exceptions ?? []).map((exception) => ({
          group: exception.group,
          project_id: exception.project_id,
          resource_id: exception.resource_id,
          access: RESTRICTION[exception.access],
        })),
      });
    }
    if (update.access) {
      await call('PUT', `/agents/${encodeURIComponent(id)}/scopes`, {
        scopes: SCOPES[update.access],
      });
    }
    const found = (await agentsPage()).items.find((agent) => agent.id === id);
    if (!found) throw new RouteError('agent.not_found', 'That agent is no longer connected.');
    return found;
  });

  // Coding tools on this computer asking to connect, and projects coming
  // through the private link. Both are answered on Hoista's consent page;
  // remote ones only there.
  router.on('GET', '/agents/address', () => call<AgentAddress | null>('GET', '/agents/address'));
  router.on('GET', '/agents/link', () => call<AgentLink>('GET', '/agents/link'));
  router.on('DELETE', '/agents/link', () => call<AgentLink>('DELETE', '/agents/link'));
  router.on('PUT', '/agents/link', ({ body }) => call<AgentLink>('PUT', '/agents/link', body));
  router.on('GET', '/agents/requests', async (): Promise<Page<ConnectionRequest>> => {
    const [answer, approvals, agents, address] = await Promise.all([
      call<{ items: EnrollmentRequest[] }>('GET', '/agents/connection-requests'),
      allPages<DaemonApproval>(call, '/approvals'),
      call<DaemonPage<DaemonAgent>>('GET', '/agents'),
      call<AgentAddress | null>('GET', '/agents/address').catch(() => null),
    ]);
    const names = new Map(agents.items.map((agent) => [agent.id, agentDisplayName(agent.name)]));
    // Hoista's own approval pages: on this computer, or a server's public ones.
    const pagesAt = address?.local ?? address?.public ?? null;
    const now = Date.now();
    const waiting = approvals.filter(
      (approval) =>
        approval.action === 'connect_agent' &&
        approval.decision === 'pending' &&
        Date.parse(approval.expires_at) > now,
    );
    // The code Hoista's page shows, to match it here, and where the browser
    // goes back to: a tool on this computer signs in through 127.0.0.1.
    const found = await Promise.all(
      waiting.map((approval) =>
        call<{ code: string; returns_to?: string }>(
          'GET',
          `/agents/oauth-requests/${encodeURIComponent(approval.target)}`,
        ).catch(() => null),
      ),
    );
    const remote: ConnectionRequest[] = waiting.map((approval, index) => ({
      id: `${WEB}${approval.target}`,
      claimed_name: names.get(approval.agent_id) ?? 'An agent',
      via: isLoopback(found[index]?.returns_to) ? 'this_computer' : 'link',
      // Its page sends the browser back to the agent, so it's answered there.
      answer_in: 'browser',
      access: 'look',
      created_at: approval.created_at,
      code: found[index]?.code ?? null,
      expires_at: approval.expires_at,
      page: pagesAt ? `${pagesAt}/connect/oauth-${encodeURIComponent(approval.target)}` : null,
    }));
    return {
      items: [
        ...remote,
        ...answer.items.map((request): ConnectionRequest => ({
          id: request.id,
          claimed_name: agentDisplayName(request.name),
          via: 'this_computer',
          answer_in: 'app',
          access: 'look',
          created_at: request.created_at,
          code: request.code,
          expires_at: request.expires_at,
          page: pagesAt ? `${pagesAt}/connect/${encodeURIComponent(request.id)}` : null,
          // Allowing it replaces the same tool's earlier connections here.
          replaces: agents.items.filter(
            (agent) => !agent.revoked && agent.transport === 'stdio' && agent.name === request.name,
          ).length,
        })),
      ],
      cursor: null,
    };
  });

  router.on('POST', '/agents/requests/:id', async ({ params, body }) => {
    const decision = (body ?? {}) as Decision;
    const id = params.id ?? '';
    const answer = {
      allow: decision.decision === 'allow',
      scopes: SCOPES[decision.access ?? 'look'],
      grant: decision.projects ? { projects: decision.projects } : undefined,
    };
    // A sign-in through the browser is answered only on Hoista's page there:
    // that page is what sends the browser back to the agent.
    if (id.startsWith(WEB)) {
      throw new RouteError(
        'agents.answer_in_browser',
        'Approve it on Hoista’s page in your browser, so it can send you back to the agent.',
      );
    }
    await call('POST', `/agents/connection-requests/${encodeURIComponent(id)}`, answer);
    return null;
  });
}
