import { providerEntry } from '@shared/providers';
import { useState } from 'react';
import type { ProviderTarget } from '@shared/api';
import { useProviderRead, useRunProviderOperation } from '@ui/api/providerOperations';
import { openExternal } from '@ui/api/system';
import { Button } from '@ui/components/Button';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { FixedValue, SettingGroup, SettingRow } from '@ui/components/SettingRow';
import { SkeletonRows } from '@ui/components/Skeleton';
import { showToast } from '@ui/lib/toast';
import { timeAgo } from '@ui/lib/format';
import styles from './AlertsSection.module.css';
import shared from './account.module.css';
import { channelWord, words } from './accountWords';

interface Rule {
  id: string;
  eventTypes?: string[];
  severities?: string[] | null;
  projectId?: string | null;
  serviceId?: string | null;
}

interface Delivery {
  node: {
    id: string;
    createdAt: string;
    status?: string;
    type?: string;
    notificationInstance?: {
      eventType?: string;
      status?: string;
      resolvedAt?: string | null;
      projectId?: string | null;
    };
  };
}

export function AlertsSection({
  target,
  name,
  service,
}: {
  target: ProviderTarget;
  name: string;
  service: string;
}) {
  const rules = useProviderRead<Rule[]>(target, 'notificationRules');
  const deliveries = useProviderRead<{ edges?: Delivery[] }>(target, 'notificationDeliveries', {
    first: 8,
  });
  const run = useRunProviderOperation(target);
  const [removing, setRemoving] = useState<string | null>(null);

  return (
    <>
      <SettingGroup
        title="Rules"
        description={`${name} sends these itself, by email, in its app or to a webhook. Hoista also shows them in Needs attention.`}
        note={
          <>
            New rules and channels are set up in {name}.{' '}
            <button
              type="button"
              className={shared.inline}
              onClick={() => {
                void openExternal(
                  providerEntry(service)?.dashboard?.('') ??
                    'https://railway.com/account/notifications',
                );
              }}
            >
              Open alerts in {name} ↗
            </button>
          </>
        }
      >
        {rules.isPending && <SkeletonRows count={3} />}
        {!rules.isPending && (rules.data ?? []).length === 0 && (
          <SettingRow label="No rules yet" hint={`${name} sends nothing until you add one.`} />
        )}
        {(rules.data ?? []).map((rule) => (
          <SettingRow
            key={rule.id}
            label={(rule.eventTypes ?? []).map(words).join(', ') || 'Any event'}
            hint={[
              rule.serviceId ? 'One service' : rule.projectId ? 'One project' : 'Every project',
              (rule.severities ?? []).length > 0
                ? (rule.severities ?? []).map(words).join(', ')
                : null,
            ]
              .filter(Boolean)
              .join(' · ')}
          >
            {removing === rule.id ? (
              <span className={styles.confirm}>
                <Button
                  variant="danger"
                  busy={run.isPending}
                  busyLabel="Removing"
                  onClick={() => {
                    run.mutate(
                      { name: 'notificationRuleDelete', args: { id: rule.id } },
                      {
                        onSuccess: () => {
                          showToast(`Alert rule removed at ${name}`);
                        },
                        onSettled: () => {
                          setRemoving(null);
                        },
                      },
                    );
                  }}
                >
                  Remove
                </Button>
                <Button
                  variant="ghost"
                  onClick={() => {
                    setRemoving(null);
                  }}
                >
                  Keep
                </Button>
              </span>
            ) : (
              <Button
                variant="quietDanger"
                onClick={() => {
                  setRemoving(rule.id);
                }}
              >
                Remove
              </Button>
            )}
          </SettingRow>
        ))}
      </SettingGroup>
      <ErrorMessage error={run.error ?? rules.error} />
      <SettingGroup title="Recently">
        {deliveries.isPending && <SkeletonRows count={2} />}
        {!deliveries.isPending && (deliveries.data?.edges ?? []).length === 0 && (
          <SettingRow label="No alerts yet" />
        )}
        {(deliveries.data?.edges ?? []).map(({ node }) => (
          <SettingRow
            key={node.id}
            label={words(node.notificationInstance?.eventType ?? 'Alert')}
            hint={`${timeAgo(node.createdAt)}${
              node.notificationInstance?.status
                ? ` · ${words(node.notificationInstance.status)}`
                : ''
            }`}
          >
            {node.type && <FixedValue>{channelWord(node.type)}</FixedValue>}
          </SettingRow>
        ))}
      </SettingGroup>
    </>
  );
}
