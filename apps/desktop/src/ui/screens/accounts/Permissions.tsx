import { serviceName, providerEntry } from '@shared/providers';
import type { Account, ProviderOperation, ProviderRisk } from '@shared/api';
import { useProviderOperations } from '@ui/api/providerOperations';
import { FixedValue, SettingGroup, SettingRow } from '@ui/components/SettingRow';
import { SkeletonRows } from '@ui/components/Skeleton';
import { plural } from '@ui/lib/format';

/** What each area of the provider's API allows, in the order the provider lists them. */
function areasOf(
  operations: ProviderOperation[],
): { area: string; risks: Set<ProviderRisk>; count: number }[] {
  const areas = new Map<string, { risks: Set<ProviderRisk>; count: number }>();
  for (const operation of operations) {
    const found = areas.get(operation.area) ?? { risks: new Set<ProviderRisk>(), count: 0 };
    found.risks.add(operation.risk);
    found.count += 1;
    areas.set(operation.area, found);
  }
  return [...areas.entries()].map(([area, value]) => ({ area, ...value }));
}

/** The highest access an area needs, in the provider's own words. */
function allows(risks: Set<ProviderRisk>, service: string): string {
  const words = providerEntry(service)?.access ?? {
    read: 'Read',
    write: 'Read and write',
    delete: 'Read, write and delete',
  };
  return risks.has('destroy') ? words.delete : risks.has('change') ? words.write : words.read;
}

/** "3 actions", and that it deletes when the provider's words don't say so. */
function actionsHint(item: { risks: Set<ProviderRisk>; count: number }, service: string): string {
  const words = providerEntry(service)?.access;
  const hidden = item.risks.has('destroy') && words !== undefined && words.delete === words.write;
  return `${plural(item.count, 'action')}${hidden ? ' · includes deleting' : ''}`;
}

/**
 * Every permission, one row each. A provider without scoped tokens (Railway)
 * lets a token do whatever the account can, so the list is what Hoista can
 * do with it, by area of the provider's API. The checks run when the token was
 * added follow. Agents with Ask for approval ask before deleting.
 */
export function Permissions({ account }: { account: Account }) {
  const name = serviceName(account.service);
  const operations = useProviderOperations({ kind: 'account', id: account.id });
  const areas = areasOf(operations.data ?? []);

  return (
    <>
      {(operations.isPending || areas.length > 0) && (
        <SettingGroup
          title="What it allows"
          description={`${name} tokens aren’t limited to parts of the account, so this is everything Hoista can do at ${name} with it. Agents with Ask for approval ask you before deleting.`}
        >
          {operations.isPending && <SkeletonRows count={4} />}
          {areas.map((item) => (
            <SettingRow key={item.area} label={item.area} hint={actionsHint(item, account.service)}>
              <FixedValue>{allows(item.risks, account.service)}</FixedValue>
            </SettingRow>
          ))}
        </SettingGroup>
      )}
      {account.scopes.length + account.missing_scopes.length > 0 && (
        <SettingGroup title="Checked when it was added">
          {account.scopes.map((scope) => {
            const check = scopeResources(scope);
            return (
              <SettingRow key={scope} label={check.what} hint={check.how}>
                <FixedValue>{check.access}</FixedValue>
              </SettingRow>
            );
          })}
          {account.missing_scopes.map((scope) => (
            <SettingRow
              key={scope}
              label={scopeResources(scope).what}
              hint="The token can’t do this"
            >
              <FixedValue>Missing</FixedValue>
            </SettingRow>
          ))}
        </SettingGroup>
      )}
    </>
  );
}

/** "deployment logs and metrics: read with the same token" → what, access, how. */
function scopeResources(scope: string): { what: string; access: string; how: string | undefined } {
  const [rawWhat, ...rest] = scope.split(':');
  const after = rest.join(':').trim();
  const how =
    /\((.*)\)/.exec(after)?.[1] ??
    (after.includes(' with ') ? after.slice(after.indexOf(' with ') + 1) : undefined);
  const access =
    after
      .replace(/\(.*\)/, '')
      .replace(/ with .*/, '')
      .trim() || 'Allowed';
  const what = (rawWhat ?? scope).trim();
  return {
    what: what.charAt(0).toUpperCase() + what.slice(1),
    access: access.charAt(0).toUpperCase() + access.slice(1),
    how: how ? how.charAt(0).toUpperCase() + how.slice(1) : undefined,
  };
}
