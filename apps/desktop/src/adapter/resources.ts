/**
 * A resource's page (`GET /resources/{id}`) from the daemon's resource detail: who it
 * belongs to, what uses it, its latest recorded usage and its account.
 *
 * Only recorded facts are shown. The engine keeps no per-resource event feed
 * yet, so `events` is empty; stats appear only for measured values.
 */
import type { LogHistory, ResourceDetail as DaemonDetail, Usage } from '@engine-api';
import type { ResourceDetail, ResourceStat } from '@shared/api';
import { providerEntry, SERVICES } from '@shared/providers';
import type { Router } from '@shared/router';
import { abilitiesOf, type Abilities } from './abilities';
import type { Call } from './call';
import type { ProviderReads } from './provider';
import { toResource } from './overview';
import { MINUTE_MS, HOUR_MS, floorTo } from '@shared/time';

export function addResourceRoutes(
  router: Router,
  call: Call,
  reads: ProviderReads,
  abilities: () => Promise<Abilities> = abilitiesOf(call),
): void {
  router.on('GET', '/resources/:id', async ({ params }): Promise<ResourceDetail> => {
    const id = params.id ?? '';
    const raw = await call<DaemonDetail>('GET', `/resources/${encodeURIComponent(id)}/details`);
    const detail = toResourceDetail(raw);
    if ((await abilities()).can(detail.resource.provider, 'provider_deploys')) {
      try {
        const provider = await reads.forResource(id);
        const name = SERVICES[detail.resource.provider].name;
        // The provider's own deploys are this resource's events.
        detail.events = provider.deployments.slice(0, 8).map((deployment) => ({
          at: deployment.created_at,
          title: `${deployment.state === 'failed' ? 'Deploy failed' : deployment.state === 'succeeded' ? 'Deployed' : 'Deploying'} ${(deployment.commit ?? deployment.id).slice(0, 7)}${deployment.message ? ` · ${deployment.message}` : ''} on ${name}`,
          tone:
            deployment.state === 'failed'
              ? 'bad'
              : deployment.state === 'succeeded'
                ? 'good'
                : 'neutral',
        }));
        // What this resource uses, from its variables that refer to another service.
        for (const reference of provider.references) {
          if (!detail.settings.some((row) => row.label === reference.variable)) {
            detail.settings.push({
              label: reference.variable,
              value: `Refers to ${reference.service} on ${name}`,
            });
          }
        }
      } catch {
        // A provider read that fails leaves the recorded facts on their own.
      }
    }
    // A provider whose console needs names Hoista doesn't keep: its newest
    // deployment's page leads to it.
    const consoleFrom = providerEntry(detail.resource.provider)?.consoleFrom;
    if (consoleFrom && (await abilities()).can(detail.resource.provider, 'provider_details')) {
      try {
        const page = (await reads.forResource(id)).deployments.find(
          (item) => item.provider_page,
        )?.provider_page;
        const url = page ? consoleFrom(page) : null;
        if (url) detail.console_url = url;
      } catch {
        // The dashboard link stays.
      }
    }
    // A provider without deploys to list: its own newest log lines are what happened.
    const can = await abilities();
    if (
      !can.can(detail.resource.provider, 'provider_deploys') &&
      can.can(detail.resource.provider, 'logs') &&
      raw.project
    ) {
      try {
        const to = new Date(floorTo(Date.now(), MINUTE_MS) + MINUTE_MS);
        const from = new Date(to.getTime() - 24 * HOUR_MS);
        const history = await call<LogHistory>(
          'GET',
          `/projects/${encodeURIComponent(raw.project.id)}/logs/history?${new URLSearchParams({
            resource: id,
            from: from.toISOString(),
            to: to.toISOString(),
            query: '',
          }).toString()}`,
        );
        detail.events = [...history.items]
          .reverse()
          .slice(0, 8)
          .map((entry) => ({
            at: entry.timestamp ?? entry.observed_at,
            title: entry.message.length > 140 ? `${entry.message.slice(0, 139)}…` : entry.message,
            tone: entry.level === 'error' ? 'bad' : entry.level === 'warn' ? 'warn' : 'neutral',
          }));
      } catch {
        // No logs to show leaves the recorded facts on their own.
      }
    }
    return detail;
  });
}

const STATUS_NOTE: Partial<Record<ResourceDetail['resource']['status'], string>> = {
  sleeping: 'Asleep. It wakes on the next request.',
  idle: 'No requests lately.',
  paused: 'Paused. It does not serve requests until resumed.',
  unknown: 'The provider does not report this resource’s status to Hoista yet.',
};

export function toResourceDetail(detail: DaemonDetail): ResourceDetail {
  const resource = toResource(detail.resource);
  const provider = SERVICES[detail.resource.provider].name;
  return {
    resource,
    summary: [provider, resource.product, detail.account.label, detail.resource.region]
      .filter((value) => value)
      .join(' · '),
    status_note: STATUS_NOTE[detail.resource.status] ?? null,
    stats: statsFrom(detail.latest_usage, detail.resource.provider),
    used_by: detail.used_by.map((user) => ({
      resource_id: user.resource.id,
      name: user.resource.name,
      provider: user.resource.provider,
      via: user.link.via,
    })),
    events: [],
    settings: [
      { label: 'Account', value: `${detail.account.label} · ${detail.account.external_id}` },
      { label: 'Provider ID', value: detail.resource.provider_ref },
      ...(detail.resource.region ? [{ label: 'Region', value: detail.resource.region }] : []),
      {
        label: 'Hoista',
        value: detail.resource.mode === 'watching' ? 'Watching (read-only)' : 'Managing',
      },
    ],
    console_url: consoleUrl(detail),
  };
}

function statsFrom(
  usage: Usage | null,
  provider: DaemonDetail['resource']['provider'],
): ResourceStat[] {
  if (!usage) return [];
  const words = providerEntry(provider)?.words;
  const note = `on ${usage.day}`;
  const stats: ResourceStat[] = [];
  if (usage.requests !== null) {
    stats.push({ label: 'Requests', value: usage.requests.toLocaleString('en-US'), note });
  }
  if (usage.errors !== null) {
    stats.push({ label: 'Errors', value: usage.errors.toLocaleString('en-US'), note });
  }
  if (usage.compute_seconds !== null) {
    stats.push({
      label: words?.compute ?? 'Compute',
      value: computeTime(usage.compute_seconds, words?.vcpu === true),
      note,
    });
  }
  // Nothing on disk is no fact worth a row: a service without a volume.
  if (usage.storage_bytes !== null && usage.storage_bytes > 0) {
    stats.push({
      label: words?.storage ?? 'Storage',
      value:
        usage.storage_bytes >= 1e9
          ? `${(usage.storage_bytes / 1e9).toFixed(2)} GB`
          : `${(usage.storage_bytes / 1e6).toFixed(1)} MB`,
      note,
    });
  }
  return stats;
}

/**
 * Compute time in the smallest unit that still shows a number. Where a provider
 * measures CPU in vCPUs, compute is vCPU time; a nearly idle service uses
 * seconds of it a day, which minutes would round to 0.0.
 */
export function computeTime(seconds: number, vcpu: boolean): string {
  const unit = vcpu ? 'vCPU-' : '';
  if (seconds < 1) return vcpu ? 'Under 1 vCPU-sec' : 'Under 1 sec';
  if (seconds < 120) return `${String(Math.round(seconds))} ${unit}sec`;
  const minutes = seconds / 60;
  return `${minutes < 10 ? minutes.toFixed(1) : Math.round(minutes).toLocaleString('en-US')} ${unit}min`;
}

/** The provider's console for this resource, or its dashboard when the deep link is not known. */
function consoleUrl(detail: DaemonDetail): string {
  return (
    providerEntry(detail.resource.provider)?.console?.(
      detail.resource.provider_ref,
      detail.account.external_id,
    ) ?? ''
  );
}
