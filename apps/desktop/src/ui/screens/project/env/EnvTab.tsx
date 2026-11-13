/**
 * Environment: the keys and settings the app reads, which resource uses each,
 * and what kind it is: linked by Hoista, a secret, a plain setting, or read
 * by the code but not set. Values go straight to the provider and are never
 * shown again, here or to agents.
 */
import { KeyRound, Lock, MoreVertical } from 'lucide-react';
import { useState } from 'react';
import type { Resource, Secret } from '@shared/api';
import { providerEntry, serviceName } from '@shared/providers';
import { useCan } from '@ui/api/providers';
import { useProject, useDeleteSecret, useSecrets } from '@ui/api/projects';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { Button } from '@ui/components/Button';
import { Panel } from '@ui/components/Panel';
import { QueryState } from '@ui/components/QueryState';
import { SkeletonRows } from '@ui/components/Skeleton';
import { PillSelect, type Option } from '@ui/components/Select';
import { cx } from '@ui/lib/cx';
import { usePopover } from '@ui/lib/usePopover';
import { useRequiredParam } from '@ui/lib/useRequiredParam';
import { ProjectTab, tabStyles } from '../ProjectTab';
import { resourceKindLabel } from '../resourceKinds';
import { PasteVariablesDialog } from './PasteVariablesDialog';
import { VariableDialog } from './VariableDialog';
import styles from './EnvTab.module.css';
import { ResourceSelect } from '@ui/components/ResourceSelect';

/** Which variable the dialog is open for: a new one, or one being set or replaced. */
type Editing = { mode: 'add' } | { mode: 'edit'; secret: Secret } | null;

type Kind = 'shared' | 'linked' | 'secret' | 'public' | 'plain' | 'missing';
type ShowFilter = Kind | null;

const SHOW_CHOICES: Option<ShowFilter>[] = [
  { value: null, label: 'All' },
  { value: 'secret', label: 'Secrets' },
  { value: 'public', label: 'Public' },
  { value: 'plain', label: 'Plain' },
  { value: 'linked', label: 'Linked' },
  { value: 'shared', label: 'Shared' },
  { value: 'missing', label: 'Not set' },
];

/** Names a framework builds into the browser bundle (Next.js, Vite, Astro, Expo, Nuxt). */
const PUBLIC_PREFIXES = ['NEXT_PUBLIC_', 'VITE_', 'PUBLIC_', 'EXPO_PUBLIC_', 'NUXT_PUBLIC_'];

function kindOf(secret: Secret): Kind {
  if (secret.missing) return 'missing';
  if (secret.shared) return 'shared';
  if (secret.managed_by || secret.links_to) return 'linked';
  // Frameworks put these in the code sent to every visitor's browser: they
  // can't be secret, whatever the provider stores them as.
  if (PUBLIC_PREFIXES.some((prefix) => secret.name.startsWith(prefix))) return 'public';
  return secret.preview === null ? 'secret' : 'plain';
}

/** `pinned` shows one resource's variables only, on that resource's page. */
export function EnvTab({ pinned }: { pinned?: string } = {}) {
  const projectId = useRequiredParam('projectId');
  const secrets = useSecrets(projectId);
  const project = useProject(projectId).data;
  const resources = project?.resources ?? [];
  // Watched projects: names come from the provider; values are set and removed there.
  const watching = project?.project.mode === 'watching';
  const [editing, setEditing] = useState<Editing>(null);
  const [pasting, setPasting] = useState(false);
  const [removing, setRemoving] = useState<Secret | null>(null);
  const [chosenId, setResourceId] = useState<string | null>(null);
  const resourceId = pinned ?? chosenId;
  const [show, setShow] = useState<ShowFilter>(null);

  const all = secrets.data?.items ?? [];
  const can = useCan();
  // Watched: every resource whose provider keeps variables, even before it has any.
  const variableResources = watching
    ? resources.filter((resource) => {
        const on = providerEntry(resource.provider)?.variables?.on;
        return can(resource.provider, 'provider_variables') && (!on || on.includes(resource.kind));
      })
    : [];
  const usedResources = resources.filter(
    (resource) =>
      variableResources.includes(resource) ||
      all.some((secret) => secret.resource_id === resource.id),
  );
  const shown = all.filter(
    (secret) =>
      (resourceId === null || secret.resource_id === resourceId) &&
      (show === null || kindOf(secret) === show),
  );
  const missing = all.filter((secret) => secret.missing);
  // Only filters that can match something here. A watched project's provider keeps
  // every value hidden and doesn't say which are secret, so "Secrets" isn't offered.
  const kinds = new Set(all.map(kindOf));
  const showChoices = SHOW_CHOICES.filter(
    (choice) =>
      choice.value === null ||
      (kinds.has(choice.value) && !(watching && choice.value === 'secret')),
  );
  // The providers the note talks about: the chosen resource's, or every one listed.
  const chosen = resources.find((resource) => resource.id === resourceId);
  const named = [
    ...new Set(
      (chosen ? [chosen] : usedResources.length ? usedResources : resources).map((resource) =>
        serviceName(resource.provider),
      ),
    ),
  ];
  const provider =
    named.length === 0
      ? 'your provider'
      : named.length === 1
        ? (named[0] ?? 'your provider')
        : `${named.slice(0, -1).join(', ')} and ${named.at(-1) ?? ''}`;
  const where = named.length > 1 && !chosen ? 'their provider' : provider;

  return (
    <ProjectTab>
      <div className={tabStyles.intro}>
        <div className={tabStyles.controls}>
          {!pinned && usedResources.length > 1 && (
            <ResourceSelect
              all
              resources={usedResources}
              value={resourceId}
              onChange={setResourceId}
            />
          )}
          {showChoices.length > 2 && (
            <PillSelect label="Show" value={show} options={showChoices} onChange={setShow} />
          )}
          <span className={styles.note}>
            <Lock size={13} aria-hidden="true" />
            {watching
              ? `Read from ${provider}. Values go straight to ${where}; Hoista never keeps them.`
              : `Secrets go straight to ${provider}. Hoista and your agents never keep them.`}
          </span>
        </div>
        <span className={styles.actions}>
          {watching && (
            <Button
              large
              onClick={() => {
                setPasting(true);
              }}
            >
              Paste many
            </Button>
          )}
          <Button
            variant="accent"
            large
            onClick={() => {
              setEditing({ mode: 'add' });
            }}
          >
            Add a variable
          </Button>
        </span>
      </div>

      <Panel className={tabStyles.tableWrap}>
        <QueryState query={secrets} skeleton={<SkeletonRows count={4} columns={3} />}>
          {() => (
            <SecretsTable
              secrets={shown}
              resources={resources}
              watching={watching}
              empty={
                all.length === 0
                  ? watching
                    ? `${provider} has no variables for this project yet.`
                    : 'No variables yet.'
                  : 'No variables match.'
              }
              onEdit={(secret) => {
                setEditing({ mode: 'edit', secret });
              }}
              onRemove={
                watching
                  ? (secret) => {
                      setRemoving(secret);
                    }
                  : undefined
              }
            />
          )}
        </QueryState>
      </Panel>

      {missing.map((secret) => (
        <div key={secret.name} className={styles.missing}>
          <KeyRound size={14} aria-hidden="true" className={styles.missingIcon} />
          <span>
            <b className="mono">{secret.name}</b> is read
            {secret.read_in && (
              <>
                {' '}
                in <span className="mono">{secret.read_in}</span>
              </>
            )}{' '}
            but isn’t set. The service runs without it.
          </span>
          <Button
            large
            onClick={() => {
              setEditing({ mode: 'edit', secret });
            }}
          >
            Add it
          </Button>
        </div>
      ))}

      {removing && (
        <RemoveVariableDialog
          projectId={projectId}
          secret={removing}
          provider={provider}
          onClose={() => {
            setRemoving(null);
          }}
        />
      )}

      {pasting && (
        <PasteVariablesDialog
          projectId={projectId}
          resources={variableResources}
          provider={provider}
          onClose={() => {
            setPasting(false);
          }}
        />
      )}

      {editing && (
        <VariableDialog
          projectId={projectId}
          resources={variableResources}
          provider={provider}
          existing={editing.mode === 'edit' ? editing.secret : null}
          onClose={() => {
            setEditing(null);
          }}
        />
      )}
    </ProjectTab>
  );
}

/** Each kind's pill; a kind without a colour of its own uses the plain pill. */
const KIND_PILLS: Record<Kind, { label: string; className?: string }> = {
  linked: { label: 'Linked' },
  shared: { label: 'Shared' },
  secret: { label: 'Secret', className: styles.pillDark },
  public: { label: 'Public' },
  plain: { label: 'Plain' },
  missing: { label: 'Code reads it · not set', className: styles.pillBad },
};

function SecretsTable({
  secrets,
  resources,
  watching,
  empty,
  onEdit,
  onRemove,
}: {
  secrets: Secret[];
  resources: Resource[];
  /** The provider keeps every value hidden and doesn't say which are secret. */
  watching: boolean;
  /** What to say when there's nothing to list. */
  empty: string;
  onEdit: (secret: Secret) => void;
  /** Watched projects: remove a variable at the provider, after asking. */
  onRemove?: (secret: Secret) => void;
}) {
  if (secrets.length === 0) {
    return <p className={styles.empty}>{empty}</p>;
  }

  return (
    <table className={tabStyles.table}>
      <thead>
        <tr>
          <th scope="col">Name</th>
          <th scope="col">Value</th>
          <th scope="col">Used by</th>
          <th scope="col">Kind</th>
          <th scope="col" className={tabStyles.right}>
            <span className={tabStyles.visuallyHidden}>Actions</span>
          </th>
        </tr>
      </thead>
      <tbody>
        {secrets.map((secret) => {
          const kind = kindOf(secret);
          const resource = resources.find((candidate) => candidate.id === secret.resource_id);
          return (
            <tr key={secret.name}>
              <td className={cx(tabStyles.mono, tabStyles.strong)}>{secret.name}</td>
              <td className={cx(tabStyles.mono, tabStyles.muted)}>{valueOf(secret)}</td>
              <td className={tabStyles.muted}>
                {resource
                  ? `${resource.name} · ${resourceKindLabel(resource.kind)}`
                  : secret.resource_name}
              </td>
              <td>
                {!(watching && kind === 'secret') && (
                  <span className={cx(tabStyles.pill, KIND_PILLS[kind].className)}>
                    {KIND_PILLS[kind].label}
                  </span>
                )}
              </td>
              <td className={tabStyles.right}>
                {!secret.managed_by &&
                  (onRemove && kind !== 'missing' ? (
                    <VariableMenu
                      name={secret.name}
                      onEdit={() => {
                        onEdit(secret);
                      }}
                      onRemove={() => {
                        onRemove(secret);
                      }}
                    />
                  ) : (
                    <button
                      type="button"
                      className={styles.edit}
                      onClick={() => {
                        onEdit(secret);
                      }}
                    >
                      {kind === 'missing' ? 'Add' : 'Edit'}
                      <span className={tabStyles.visuallyHidden}> {secret.name}</span>
                    </button>
                  ))}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

/** What the Value column shows: who set a linked one, dots for secrets, the value if plain. */
function valueOf(secret: Secret): string {
  if (secret.missing) return '—';
  if (secret.links_to) return `From ${secret.links_to}`;
  if (secret.managed_by) return `Set by ${serviceName(secret.managed_by)}`;
  return secret.preview ?? '••••••••••••';
}

/** The row's "···": Edit, or Remove (which asks first). */
function VariableMenu({
  name,
  onEdit,
  onRemove,
}: {
  name: string;
  onEdit: () => void;
  onRemove: () => void;
}) {
  const { open, setOpen, ref } = usePopover<HTMLDivElement>();
  return (
    <div ref={ref} className={styles.menuWrap}>
      <button
        type="button"
        className={styles.more}
        aria-label={`More for ${name}`}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => {
          setOpen(!open);
        }}
      >
        <MoreVertical size={16} aria-hidden="true" />
      </button>
      {open && (
        <div className={styles.menu} role="menu">
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setOpen(false);
              onEdit();
            }}
          >
            Edit
          </button>
          <button
            type="button"
            role="menuitem"
            className={styles.danger}
            onClick={() => {
              setOpen(false);
              onRemove();
            }}
          >
            Remove
          </button>
        </div>
      )}
    </div>
  );
}

/** Are you sure? Removing a variable at the provider redeploys without it. */
function RemoveVariableDialog({
  projectId,
  secret,
  provider,
  onClose,
}: {
  projectId: string;
  secret: Secret;
  provider: string;
  onClose: () => void;
}) {
  const remove = useDeleteSecret(projectId);
  return (
    <Dialog
      title={`Remove ${secret.name}?`}
      onClose={onClose}
      actions={
        <>
          <Button large onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="danger"
            large
            busy={remove.isPending}
            busyLabel="Removing"
            onClick={() => {
              remove.mutate(
                {
                  name: secret.name,
                  resourceId: secret.resource_id,
                  shared: secret.shared === true,
                },
                { onSuccess: onClose },
              );
            }}
          >
            Remove
          </Button>
        </>
      }
    >
      <p className={styles.confirmText}>
        {secret.shared
          ? `${provider} deletes this shared variable, so no service in the project gets it any more.`
          : `${provider} deletes it from ${secret.resource_name} and redeploys without it.`}{' '}
        Code that reads it will see nothing. This can’t be undone; you’d have to set it again.
      </p>
      <ErrorMessage error={remove.error} />
    </Dialog>
  );
}
