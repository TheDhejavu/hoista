/**
 * A project's Logs tab (`GET /projects/{id}/logs`) from the daemon's retained log
 * history, read for each of the project's resources that can have logs and merged
 * oldest first (the tab shows them newest first).
 *
 * Lines are what the provider returned, already redacted by the engine. A resource
 * whose provider can't give Hoista logs is skipped; when no resource can, the route
 * says so instead of showing an empty table that looks like a quiet project.
 * The tab's Live mode re-reads this route every few seconds.
 */
import { providerEntry, SERVICES } from '@shared/providers';
import type {
  ProjectOverview,
  LogEntry,
  LogHistory,
  Resource as DaemonResource,
} from '@engine-api';
import type { LogLine, Page } from '@shared/api';
import type { Engine } from '@shared/engine';
import { RouteError, type Router } from '@shared/router';
import { query, type Call } from './call';
import { MINUTE_MS, SECOND_MS } from '@shared/time';

/** Resources that run code. Databases and buckets have no runtime logs here. */
const RUNS_CODE = new Set<DaemonResource['kind']>(['web', 'service', 'job']);
const HOUR_MINUTES = 60;
/** The window when none is asked for: a quiet service may log nothing for an hour. */
const DAY_MINUTES = 24 * HOUR_MINUTES;
/** The engine reads at most 31 days of history at once. */
const MAX_MINUTES = 31 * 24 * HOUR_MINUTES;

type Stream = NonNullable<Engine['stream']>;

/** Live lines kept per project, newest last. */
const LIVE_KEEP = 500;
/** A tail nobody has asked about for this long is closed. */
const LIVE_IDLE_MS = MINUTE_MS;
const SWEEP_EVERY_MS = 15 * SECOND_MS;

interface LiveTail {
  lines: LogLine[];
  asked: number;
  close: () => void;
}

/**
 * Live tails, one per watched resource, opened when the Logs tab is live and
 * closed when nobody has looked for a minute. Lines arrive from the provider as
 * they're written; the tab reads them from here every few seconds without
 * asking the provider again.
 */
function liveTails(stream: Stream) {
  const tails = new Map<string, LiveTail>();
  setInterval(() => {
    const now = Date.now();
    for (const [key, tail] of tails) {
      if (now - tail.asked > LIVE_IDLE_MS) {
        tail.close();
        tails.delete(key);
      }
    }
  }, SWEEP_EVERY_MS);
  return {
    /** The resource's live lines so far; opens its tail the first time. */
    read(projectId: string, resource: DaemonResource, text: string): LogLine[] {
      const key = `${projectId}~${resource.id}~${text}`;
      let tail = tails.get(key);
      if (!tail) {
        const opened: LiveTail = { lines: [], asked: Date.now(), close: () => undefined };
        opened.close = stream(
          `/projects/${encodeURIComponent(projectId)}/logs/live${query({ resource: resource.id, q: text.slice(0, 256) })}`,
          (data) => {
            const frame = data as { type?: string; entry?: LogEntry };
            if (frame.type !== 'log' || !frame.entry) return;
            opened.lines.push(toLogLine(resource, frame.entry));
            if (opened.lines.length > LIVE_KEEP)
              opened.lines.splice(0, opened.lines.length - LIVE_KEEP);
          },
          () => {
            // Ended: the next read opens a fresh tail.
            if (tails.get(key) === opened) tails.delete(key);
          },
        );
        tails.set(key, opened);
        tail = opened;
      }
      tail.asked = Date.now();
      return tail.lines;
    },
  };
}

export function addLogRoutes(router: Router, call: Call, stream?: Stream): void {
  const live = stream ? liveTails(stream) : null;
  router.on(
    'GET',
    '/projects/:id/logs',
    async ({ params, query: search }): Promise<Page<LogLine>> => {
      const id = encodeURIComponent(params.id ?? '');
      const overview = await call<ProjectOverview>('GET', `/projects/${id}`);
      const minutes = Math.min(
        Math.max(Number(search.get('since_minutes') ?? DAY_MINUTES) || DAY_MINUTES, 1),
        MAX_MINUTES,
      );
      // Rounded to 30 seconds, so repeated reads ask the provider the same question
      // and the engine can answer from what it just read (providers limit requests).
      const to = new Date(Math.floor(Date.now() / 30_000) * 30_000 + 30_000);
      const from = new Date(to.getTime() - minutes * MINUTE_MS);
      const text = search.get('q') ?? '';
      const source = search.get('source');
      // Some providers keep runtime logs for their database services too.
      // A resource's own page asks for that resource only.
      const only = search.get('resource');
      const resources = overview.resources.filter(
        (resource) =>
          (!only || resource.id === only) &&
          (RUNS_CODE.has(resource.kind) ||
            (resource.kind === 'database' &&
              providerEntry(resource.provider)?.databaseLogs === true) ||
            providerEntry(resource.provider)?.logKinds?.includes(resource.kind) === true),
      );

      const results = await Promise.allSettled(
        resources.map(async (resource) => {
          const history = await call<LogHistory>(
            'GET',
            `/projects/${id}/logs/history${query({
              resource: resource.id,
              from: from.toISOString(),
              to: to.toISOString(),
              query: text.slice(0, 256),
            })}`,
          );
          return history.items.map((entry) => toLogLine(resource, entry));
        }),
      );
      const read = results.flatMap((result) =>
        result.status === 'fulfilled' ? [result.value] : [],
      );
      if (resources.length > 0 && read.length === 0) {
        const providers = [
          ...new Set(resources.map((resource) => SERVICES[resource.provider].name)),
        ].join(' and ');
        const first = results.find((result) => result.status === 'rejected');
        throw new RouteError(
          'logs.unavailable',
          first?.status === 'rejected' && first.reason instanceof RouteError
            ? first.reason.error.message
            : `Hoista can't read logs from ${providers} yet.`,
        );
      }
      // A provider that couldn't be read says so as a line of its own, so its
      // service doesn't just look quiet.
      const now = new Date().toISOString();
      results.forEach((result, index) => {
        const resource = resources[index];
        if (result.status !== 'rejected' || !resource) return;
        const why =
          result.reason instanceof RouteError
            ? result.reason.error.message
            : 'The provider didn’t answer.';
        read.push([
          {
            ...toLogLine(resource, {
              timestamp: now,
              observed_at: now,
              level: 'error',
              source: 'hoista',
              message: `Hoista couldn’t read ${SERVICES[resource.provider].name}’s logs for ${resource.name}: ${why}`,
            }),
          },
        ]);
      });

      // Requests the provider served, where it logs each one, for the same window.
      if (!source || source === 'request') {
        const since = from.getTime();
        const requests = await Promise.allSettled(
          resources
            .filter(
              (resource) =>
                providerEntry(resource.provider)?.requests && resource.kind !== 'database',
            )
            .map((resource) => requestLines(call, resource, text)),
        );
        for (const result of requests) {
          if (result.status === 'fulfilled') {
            read.push(result.value.filter((line) => Date.parse(line.at) >= since));
          }
        }
      }

      // Live: add what the provider has streamed since, without asking it again.
      if (live && search.get('live') === '1') {
        const seen = new Set(read.flat().map((line) => `${line.at}~${line.message}`));
        for (const resource of resources.filter((item) => item.mode === 'watching')) {
          const lines = live.read(params.id ?? '', resource, text);
          read.push(lines.filter((line) => !seen.has(`${line.at}~${line.message}`)));
        }
      }

      const level = search.get('level');
      const items = read
        .flat()
        .filter((line) => (!level || line.level === level) && (!source || line.source === source))
        .sort((a, b) => a.at.localeCompare(b.at));
      return { items, cursor: null };
    },
  );
}

interface HttpLog {
  timestamp: string;
  method?: string | null;
  path?: string | null;
  httpStatus?: number | null;
  totalDuration?: number | null;
  edgeRegion?: string | null;
  provider?: string | null;
  clientUa?: string | null;
  requestId?: string | null;
  txBytes?: number | null;
  rxBytes?: number | null;
  upstreamErrors?: string | null;
  responseDetails?: string | null;
}

function bytes(count: number | null | undefined): string | null {
  if (count === null || count === undefined) return null;
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)} MB`;
  if (count >= 1_000) return `${String(Math.round(count / 1_000))} kB`;
  return `${String(count)} B`;
}

/** The newest requests a Railway service served, as log lines. */
async function requestLines(
  call: Call,
  resource: DaemonResource,
  text: string,
): Promise<LogLine[]> {
  const details = await call<{ deployments: { id: string; state: string }[] }>(
    'GET',
    `/resources/${encodeURIComponent(resource.id)}/provider`,
  );
  const live =
    details.deployments.find((item) => item.state === 'succeeded') ?? details.deployments[0];
  if (!live) return [];
  const operation = providerEntry(resource.provider)?.requests?.logs;
  if (!operation) return [];
  const rows = await call<HttpLog[]>(
    'POST',
    `/resources/${encodeURIComponent(resource.id)}/provider/operations/${encodeURIComponent(operation)}`,
    { args: { deploymentId: live.id, limit: 200, ...(text && { filter: text.slice(0, 256) }) } },
  );
  return rows.map((row) => toRequestLine(resource, row));
}

export function toRequestLine(resource: DaemonResource, row: HttpLog): LogLine {
  const status = row.httpStatus ?? 0;
  const took =
    row.totalDuration === null || row.totalDuration === undefined
      ? null
      : `${String(Math.round(row.totalDuration))}ms`;
  const facts = [
    ['Duration', took],
    ['Region', row.edgeRegion ?? null],
    ['Provider', row.provider ?? null],
    ['Sent', bytes(row.txBytes)],
    ['Received', bytes(row.rxBytes)],
    ['Client', row.clientUa ?? null],
    ['Upstream error', row.upstreamErrors || null],
    ['Details', row.responseDetails || null],
    ['Request ID', row.requestId ?? null],
  ].flatMap(([label, value]) => (value ? [{ label: label as string, value: value }] : []));
  return {
    at: row.timestamp,
    resource_id: resource.id,
    resource_name: resource.name,
    source: 'request',
    level: status >= 500 ? 'error' : status >= 400 ? 'warn' : 'info',
    message: [String(status), took].filter(Boolean).join(' · '),
    what: `${row.method ?? 'GET'} ${row.path ?? '/'}`,
    resource_kind: resource.kind,
    status,
    version: null,
    facts,
    hint: null,
  };
}

function toLogLine(resource: DaemonResource, entry: LogEntry): LogLine {
  return {
    at: entry.timestamp ?? entry.observed_at,
    resource_id: resource.id,
    resource_name: resource.name,
    source: 'project',
    level: entry.level === 'debug' ? 'info' : entry.level,
    message: entry.message,
    what: resource.name,
    resource_kind: resource.kind,
    status: null,
    version: null,
    facts: [
      { label: 'Provider', value: SERVICES[resource.provider].name },
      ...(entry.source ? [{ label: 'Stream', value: entry.source }] : []),
    ],
    hint: null,
  };
}
