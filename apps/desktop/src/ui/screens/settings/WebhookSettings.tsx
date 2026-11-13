/**
 * Settings · Webhooks: addresses Hoista sends signed events to (deploys,
 * approvals, resources going up or down), what each one gets, its recent
 * deliveries, and a way to replace its signing secret or remove it.
 */
import { useState } from 'react';
import type { Webhook, WebhookDelivery } from '@engine-api';
import { useProjects } from '@ui/api/projects';
import { ALL_ENVIRONMENTS } from '@ui/api/environments';
import {
  useAddWebhook,
  useRemoveWebhook,
  useRotateWebhook,
  useWebhookDeliveries,
  useWebhooks,
  type NewWebhook,
} from '@ui/api/webhooks';
import { Button } from '@ui/components/Button';
import { Dialog, DialogText } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { TextField } from '@ui/components/Field';
import { QueryState } from '@ui/components/QueryState';
import { SkeletonRows } from '@ui/components/Skeleton';
import { SelectField, type Option } from '@ui/components/Select';
import { SettingGroup, SettingRow, SettingGroupSkeleton } from '@ui/components/SettingRow';
import { timeAgo } from '@ui/lib/format';
import { useCopy } from '@ui/lib/useCopy';
import { RowMenu } from '@ui/components/RowMenu';
import styles from './WebhookSettings.module.css';

/** What a webhook gets, as one choice: every event, or one family. */
const EVENT_CHOICES: Option<string>[] = [
  { value: '', label: 'Every event' },
  { value: 'deploy.*', label: 'Deploys' },
  { value: 'approval.*', label: 'Approvals' },
  { value: 'resource.*', label: 'Resources going down or coming back' },
];

function eventsLabel(events: string[]): string {
  if (events.length === 0) return 'Every event';
  return events
    .map((kind) => EVENT_CHOICES.find((choice) => choice.value === kind)?.label ?? kind)
    .join(', ');
}

/** Beside the section's title in the Settings window, as Manage agents is for Agents. */
export function AddWebhookButton() {
  const [adding, setAdding] = useState(false);
  const [made, setMade] = useState<NewWebhook | null>(null);

  return (
    <>
      <Button
        variant="accent"
        onClick={() => {
          setAdding(true);
        }}
      >
        Add a webhook
      </Button>
      {adding && (
        <AddWebhookDialog
          onClose={() => {
            setAdding(false);
          }}
          onMade={(result) => {
            setAdding(false);
            setMade(result);
          }}
        />
      )}
      {made && (
        <SecretDialog
          made={made}
          onClose={() => {
            setMade(null);
          }}
        />
      )}
    </>
  );
}

export function WebhookSettings() {
  const webhooks = useWebhooks();
  const [made, setMade] = useState<NewWebhook | null>(null);

  return (
    <>
      <QueryState query={webhooks} skeleton={<SettingGroupSkeleton rows={2} />}>
        {(items) => <WebhookList webhooks={items} onSecret={setMade} />}
      </QueryState>
      {made && (
        <SecretDialog
          made={made}
          onClose={() => {
            setMade(null);
          }}
        />
      )}
    </>
  );
}

function WebhookList({
  webhooks,
  onSecret,
}: {
  webhooks: Webhook[];
  onSecret: (made: NewWebhook) => void;
}) {
  const projects = new Map(
    (useProjects({ environment: ALL_ENVIRONMENTS }).data?.items ?? []).map((project) => [
      project.id,
      project.name,
    ]),
  );
  const [open, setOpen] = useState<string | null>(null);
  const [removing, setRemoving] = useState<Webhook | null>(null);
  const rotate = useRotateWebhook();

  return (
    <SettingGroup
      title="Where Hoista sends events"
      description="Each delivery is signed with the webhook’s secret in a Hoista-Signature header, so the receiver can check it came from you. Hoista tries six times before giving up."
    >
      {webhooks.length === 0 && (
        <SettingRow
          label="No webhooks yet"
          hint="Add one to tell another service when a deploy finishes, an approval waits, or a resource goes down."
        />
      )}
      {webhooks.map((webhook) => (
        <div key={webhook.id}>
          <SettingRow
            label={webhook.url}
            hint={`${eventsLabel(webhook.events)} · ${webhook.project_id ? (projects.get(webhook.project_id) ?? webhook.project_id) : 'every project'} · added ${timeAgo(webhook.created_at)}`}
          >
            <Button
              variant="ghost"
              onClick={() => {
                setOpen(open === webhook.id ? null : webhook.id);
              }}
            >
              {open === webhook.id ? 'Hide deliveries' : 'Deliveries'}
            </Button>
            {rotate.isPending && rotate.variables === webhook.id && (
              <span className={styles.busy} role="status">
                Making a new secret
              </span>
            )}
            <RowMenu
              label={`More for ${webhook.url}`}
              items={[
                {
                  label: 'New secret',
                  disabled: rotate.isPending,
                  onSelect: () => {
                    rotate.mutate(webhook.id, { onSuccess: onSecret });
                  },
                },
                {
                  label: 'Remove',
                  danger: true,
                  onSelect: () => {
                    setRemoving(webhook);
                  },
                },
              ]}
            />
          </SettingRow>
          {open === webhook.id && <Deliveries id={webhook.id} />}
        </div>
      ))}
      <ErrorMessage error={rotate.error} />
      {removing && (
        <RemoveDialog
          webhook={removing}
          onClose={() => {
            setRemoving(null);
          }}
        />
      )}
    </SettingGroup>
  );
}

const STATE: Record<WebhookDelivery['state'], string> = {
  pending: 'Waiting to try again',
  delivered: 'Delivered',
  failed: 'Gave up',
};

function Deliveries({ id }: { id: string }) {
  const deliveries = useWebhookDeliveries(id);
  return (
    <QueryState query={deliveries} skeleton={<SkeletonRows count={2} columns={3} />}>
      {(items) =>
        items.length === 0 ? (
          <SettingRow label="Nothing sent yet" hint="Deliveries show here as events happen." />
        ) : (
          <>
            {items.slice(0, 20).map((delivery) => (
              <SettingRow
                key={delivery.id}
                label={`${delivery.event_kind} · ${STATE[delivery.state]}`}
                hint={[
                  delivery.last_status !== null ? `Answered ${String(delivery.last_status)}` : null,
                  delivery.last_error,
                  `${String(delivery.attempts)} ${delivery.attempts === 1 ? 'try' : 'tries'}`,
                  delivery.state === 'pending' ? `next ${timeAgo(delivery.next_attempt_at)}` : null,
                ]
                  .filter(Boolean)
                  .join(' · ')}
              />
            ))}
          </>
        )
      }
    </QueryState>
  );
}

function AddWebhookDialog({
  onClose,
  onMade,
}: {
  onClose: () => void;
  onMade: (made: NewWebhook) => void;
}) {
  const [url, setUrl] = useState('');
  const [events, setEvents] = useState('');
  const [projectId, setProjectId] = useState<string | null>(null);
  const add = useAddWebhook();
  const projects = useProjects({ sort: 'name', environment: ALL_ENVIRONMENTS }).data?.items ?? [];
  const projectChoices: Option<string | null>[] = [
    { value: null, label: 'Every project' },
    ...projects.map((project) => ({ value: project.id, label: project.name })),
  ];

  return (
    <Dialog
      title="Add a webhook"
      eyebrow="Settings · Webhooks"
      onClose={onClose}
      actions={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="accent"
            disabled={!url.trim() || add.isPending}
            onClick={() => {
              add.mutate(
                { url: url.trim(), events: events ? [events] : [], project_id: projectId },
                { onSuccess: onMade },
              );
            }}
          >
            {add.isPending ? 'Adding' : 'Add webhook'}
          </Button>
        </>
      }
    >
      <TextField
        label="Address"
        value={url}
        placeholder="https://example.com/hoista"
        hint="A public https:// address. Not this computer or a private network."
        onChange={setUrl}
      />
      <SelectField label="Send" value={events} options={EVENT_CHOICES} onChange={setEvents} />
      <SelectField label="For" value={projectId} options={projectChoices} onChange={setProjectId} />
      <ErrorMessage error={add.error} />
    </Dialog>
  );
}

/** The signing secret, shown once. */
function SecretDialog({ made, onClose }: { made: NewWebhook; onClose: () => void }) {
  const { copied, copy } = useCopy();
  return (
    <Dialog
      title="Copy the signing secret now"
      eyebrow={made.webhook.url}
      onClose={onClose}
      actions={
        <>
          <Button
            onClick={() => {
              copy(made.secret);
            }}
          >
            {copied ? 'Copied' : 'Copy secret'}
          </Button>
          <Button variant="accent" onClick={onClose}>
            Done
          </Button>
        </>
      }
    >
      <DialogText>
        Give it to the service receiving the events, so it can check each delivery came from Hoista.
        Hoista won’t show it again; if you lose it, make a new one.
      </DialogText>
      <p className="mono" style={{ wordBreak: 'break-all' }}>
        {made.secret}
      </p>
    </Dialog>
  );
}

function RemoveDialog({ webhook, onClose }: { webhook: Webhook; onClose: () => void }) {
  const remove = useRemoveWebhook();
  return (
    <Dialog
      title="Remove this webhook?"
      eyebrow={webhook.url}
      onClose={onClose}
      actions={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="danger"
            disabled={remove.isPending}
            onClick={() => {
              remove.mutate(webhook.id, { onSuccess: onClose });
            }}
          >
            {remove.isPending ? 'Removing' : 'Remove'}
          </Button>
        </>
      }
    >
      <DialogText>Hoista stops sending events there. Nothing else changes.</DialogText>
      <ErrorMessage error={remove.error} />
    </Dialog>
  );
}
