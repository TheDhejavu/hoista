/**
 * Importing a provider's projects into Hoista, as one table: the provider's account
 * in a dropdown, then its projects as groups of rows. Each group goes into a
 * Hoista project of the same name (an existing one when the name matches, a
 * new one otherwise), which its "Import into" dropdown changes. Each row has a
 * checkbox to leave it out, and can be dragged onto another group, or moved
 * from its ⋮, to go somewhere else on its own. Used when connecting a provider,
 * when adding what was found, and on the account window's Projects page.
 * Nothing changes at the provider.
 */
import { ArrowRight, GripVertical, Search } from 'lucide-react';
import { useEffect, useRef, useState, type DragEvent } from 'react';
import type {
  EmptyProviderProject,
  ProviderProject,
  ProviderProjectResource,
  ProviderId,
} from '@shared/api';
import { providerEntry, serviceName } from '@shared/providers';
import { useEnvironments } from '@ui/api/environments';
import { openExternal } from '@ui/api/system';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { RowMenu } from '@ui/components/RowMenu';
import { PillSelect, Select, type Option } from '@ui/components/Select';
import { cx } from '@ui/lib/cx';
import { resourceKindLabel, runsCode } from '@ui/screens/project/resourceKinds';
import { ServiceLogo } from '@ui/screens/accounts/ServiceLogo';
import styles from './ProjectPicker.module.css';
import { type Picker, type Target, sameName, targetKey } from './usePicker';

type Category = 'service' | 'data' | 'domain';

/** Services run code; databases and storage hold data; domains and email senders are names. */
function categoryOf(kind: ProviderProjectResource['kind']): Category {
  if (runsCode(kind)) return 'service';
  return kind === 'database' || kind === 'bucket' ? 'data' : 'domain';
}

const TYPES: Option<Category | null>[] = [
  { value: null, label: 'All' },
  { value: 'service', label: 'Services' },
  { value: 'data', label: 'Databases & storage' },
  { value: 'domain', label: 'Domains & email' },
];

const STATUSES: Option<'in' | 'out' | null>[] = [
  { value: null, label: 'All' },
  { value: 'in', label: 'Imported' },
  { value: 'out', label: 'Not imported' },
];

function stateOf(ids: string[], ticked: Set<string>): 'on' | 'some' | 'off' {
  const on = ids.filter((id) => ticked.has(id)).length;
  return on === 0 ? 'off' : on === ids.length ? 'on' : 'some';
}

function Box({
  state,
  label,
  onChange,
}: {
  state: 'on' | 'some' | 'off';
  label: string;
  onChange: (on: boolean) => void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (ref.current) ref.current.indeterminate = state === 'some';
  }, [state]);
  return (
    <input
      ref={ref}
      type="checkbox"
      className={styles.box}
      aria-label={label}
      checked={state === 'on'}
      onChange={() => {
        onChange(state !== 'on');
      }}
    />
  );
}

const DRAG = 'text/hoista-resource';

export function ProjectPicker({
  picker,
  provider,
  empty = [],
  everyAccount = false,
}: {
  picker: Picker;
  provider: ProviderId;
  /** The provider's projects with nothing in them yet: listed, nothing to import. */
  empty?: EmptyProviderProject[];
  /**
   * Every account's projects at once, each account under a heading with its
   * provider's logo, instead of one account picked from a dropdown. For
   * first-run setup, where everything just connected is shown together.
   */
  everyAccount?: boolean;
}) {
  const entry = providerEntry(provider);
  const providerName = serviceName(provider);
  const environments = useEnvironments().data ?? [];
  const [query, setQuery] = useState('');
  const [type, setType] = useState<Category | null>(null);
  const [status, setStatus] = useState<'in' | 'out' | null>(null);
  const [dropOn, setDropOn] = useState<string | null>(null);
  const [moving, setMoving] = useState<{
    resource: ProviderProjectResource;
    project: ProviderProject;
  } | null>(null);

  const words = query.trim().toLowerCase();
  const groups = picker.projects
    .filter((project) => everyAccount || project.account.id === picker.workspace)
    .map((project) => ({
      project,
      rows: project.resources.filter(
        (resource) =>
          (type === null || categoryOf(resource.kind) === type) &&
          (status === null || (status === 'in') === picker.ticked.has(resource.id)) &&
          (!words ||
            resource.name.toLowerCase().includes(words) ||
            project.name.toLowerCase().includes(words) ||
            (resource.url ?? '').toLowerCase().includes(words)),
      ),
    }))
    .filter((group) => group.rows.length > 0);
  // With every account shown, each account's projects sit under its heading.
  const accounts = everyAccount
    ? [...new Set(groups.map(({ project }) => project.account.id))].map((id) => ({
        id,
        groups: groups.filter(({ project }) => project.account.id === id),
      }))
    : [{ id: picker.workspace, groups }];

  // Where a group can go: a new project of its name, or any Hoista project,
  // listed under "New project" and "Existing projects" headings.
  const targetOptions = (project: ProviderProject): Option<string>[] => {
    const named = picker.existing.find((item) => sameName(item.name, project.name));
    const current = picker.groupTarget(project);
    const fresh = [
      ...(named ? [] : [project.name]),
      ...(current.kind === 'new' && !sameName(current.name, project.name) ? [current.name] : []),
    ];
    const existing = [...picker.existing];
    // The current choice is always listed, even before the project list has loaded.
    if (current.kind === 'existing' && !existing.some((item) => item.id === current.id)) {
      existing.unshift({ id: current.id, name: current.name });
    }
    return [
      ...fresh.map((name) => ({ value: `new:${name}`, label: name, group: 'New project' })),
      ...existing.map((item) => ({
        value: `existing:${item.id}`,
        label: item.name,
        group: 'Existing projects',
      })),
    ];
  };
  const parseTarget = (project: ProviderProject, key: string): Target => {
    if (key.startsWith('existing:')) {
      const id = key.slice('existing:'.length);
      const name = picker.existing.find((item) => item.id === id)?.name ?? id;
      return { kind: 'existing', id, name };
    }
    return { kind: 'new', name: key.slice('new:'.length) || project.name };
  };
  const label = (target: Target) =>
    target.kind === 'existing' ? target.name : `${target.name} (new)`;

  function onDrop(event: DragEvent, project: ProviderProject): void {
    event.preventDefault();
    setDropOn(null);
    const id = event.dataTransfer.getData(DRAG);
    if (!id) return;
    const from = picker.projects.find((item) => item.resources.some((r) => r.id === id));
    // Dropped back on its own group: it goes with the group again.
    picker.moveResource(id, from?.key === project.key ? null : picker.groupTarget(project));
  }

  return (
    <div className={styles.picker}>
      {!everyAccount && (
        <>
          <div className={styles.tools}>
            {picker.workspaces.length > 0 && (
              <PillSelect
                label={entry?.accountWord ?? 'Account'}
                value={picker.workspace}
                options={picker.workspaces.map((item) => ({ value: item.id, label: item.label }))}
                onChange={picker.setWorkspace}
              />
            )}
            {environments.length > 1 && (
              <>
                <ArrowRight size={14} className={styles.arrow} aria-label="into" />
                <PillSelect
                  label="Hoista environment"
                  value={picker.environment}
                  options={environments.map((item) => ({ value: item.id, label: item.name }))}
                  onChange={picker.setEnvironment}
                />
              </>
            )}
          </div>
          <div className={styles.tools}>
            <label className={styles.search}>
              <Search size={13} aria-hidden="true" />
              <input
                type="search"
                placeholder="Search resources"
                value={query}
                onChange={(event) => {
                  setQuery(event.target.value);
                }}
              />
            </label>
            <PillSelect label="Type" value={type} options={TYPES} onChange={setType} />
            <PillSelect label="Status" value={status} options={STATUSES} onChange={setStatus} />
          </div>
        </>
      )}

      <div className={styles.frame}>
        <table className={styles.table}>
          <thead>
            <tr>
              <th scope="col" className={styles.boxCol}>
                <span className="visually-hidden">Import</span>
              </th>
              <th scope="col">Name</th>
              <th scope="col">Type</th>
              <th scope="col" className={styles.whereCol}>
                Hoista project
              </th>
            </tr>
          </thead>
          {accounts.map((account) => [
            everyAccount && account.groups[0] && (
              <tbody key={`account:${account.id}`}>
                <tr className={styles.account}>
                  <td colSpan={4}>
                    <span className={styles.accountName}>
                      <ServiceLogo
                        id={account.groups[0].project.provider}
                        name={serviceName(account.groups[0].project.provider)}
                        size={24}
                      />
                      {serviceName(account.groups[0].project.provider)} ·{' '}
                      {account.groups[0].project.account.label}
                    </span>
                  </td>
                </tr>
              </tbody>
            ),
            ...account.groups.map(({ project, rows }) => {
              const ids = project.resources.map((resource) => resource.id);
              const target = picker.groupTarget(project);
              const own = providerEntry(project.provider);
              const only = project.resources.length === 1 ? project.resources[0] : undefined;
              return (
                <tbody key={project.key}>
                  <tr
                    className={cx(styles.group, dropOn === project.key && styles.dropping)}
                    onDragOver={(event) => {
                      if (!event.dataTransfer.types.includes(DRAG)) return;
                      event.preventDefault();
                      setDropOn(project.key);
                    }}
                    onDragLeave={() => {
                      setDropOn((current) => (current === project.key ? null : current));
                    }}
                    onDrop={(event) => {
                      onDrop(event, project);
                    }}
                  >
                    <td className={styles.boxCol}>
                      <Box
                        state={stateOf(ids, picker.ticked)}
                        label={`Import ${project.name}`}
                        onChange={(on) => {
                          picker.toggle(ids, on);
                        }}
                      />
                    </td>
                    <td colSpan={2}>
                      <b className={styles.groupName}>{project.name}</b>
                      <span className={styles.groupMeta}>
                        {own?.groupWord
                          ? `${serviceName(project.provider)} ${own.groupWord} · `
                          : // A provider without projects: say what the one resource is.
                            only && `${resourceKindLabel(only.kind)} · `}
                        {project.resources.length}{' '}
                        {project.resources.length === 1 ? 'resource' : 'resources'}
                      </span>
                      {dropOn === project.key && (
                        <span className={styles.dropHint}>Drop to move it here</span>
                      )}
                    </td>
                    <td className={styles.whereCol}>
                      <span className={styles.into}>
                        <PillSelect
                          label="Import into"
                          value={targetKey(target)}
                          options={targetOptions(project)}
                          display={(option) =>
                            option.value.startsWith('new:') ? `${option.label} (new)` : option.label
                          }
                          onChange={(key) => {
                            picker.setGroupTarget(project, parseTarget(project, key));
                          }}
                        />
                      </span>
                    </td>
                  </tr>
                  {rows.map((resource) => {
                    const on = picker.ticked.has(resource.id);
                    const where = picker.rowTarget(project, resource);
                    const apart = picker.movedApart(resource.id);
                    return (
                      <tr
                        key={resource.id}
                        className={cx(styles.row, !on && styles.off)}
                        draggable
                        onDragStart={(event) => {
                          event.dataTransfer.setData(DRAG, resource.id);
                          event.dataTransfer.effectAllowed = 'move';
                        }}
                      >
                        <td className={styles.boxCol}>
                          <span className={styles.handle}>
                            <GripVertical size={14} aria-hidden="true" />
                            <Box
                              state={on ? 'on' : 'off'}
                              label={`Import ${resource.name}`}
                              onChange={(next) => {
                                picker.toggle([resource.id], next);
                              }}
                            />
                          </span>
                        </td>
                        <td>
                          <span className={styles.name}>{resource.name}</span>
                          {resource.url && (
                            <span className={cx(styles.sub, 'mono')}>
                              {resource.url.replace(/^https?:\/\//, '')}
                            </span>
                          )}
                        </td>
                        <td>
                          <span className={cx(styles.tag, styles[categoryOf(resource.kind)])}>
                            {resourceKindLabel(resource.kind)}
                          </span>
                        </td>
                        <td className={styles.whereCol}>
                          <span className={styles.whereRow}>
                            <span className={styles.where}>
                              {on ? label(where) : 'Not imported'}
                              {on && apart && (
                                <span className={styles.apart}>Moved on its own</span>
                              )}
                            </span>
                            <RowMenu
                              label={`More for ${resource.name}`}
                              items={[
                                {
                                  label: 'Move to project',
                                  onSelect: () => {
                                    setMoving({ resource, project });
                                  },
                                },
                                ...(apart
                                  ? [
                                      {
                                        label: 'Keep with its group',
                                        onSelect: () => {
                                          picker.moveResource(resource.id, null);
                                        },
                                      },
                                    ]
                                  : []),
                              ]}
                            />
                          </span>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              );
            }),
          ])}
          {status !== 'in' &&
            type === null &&
            empty
              .filter((project) => !words || project.name.toLowerCase().includes(words))
              .map((project) => (
                <tbody key={`empty:${project.id}`}>
                  <tr className={styles.emptyGroup}>
                    <td className={styles.boxCol} />
                    <td colSpan={2}>
                      <b className={styles.groupName}>{project.name}</b>
                      <span className={styles.groupMeta}>
                        {entry?.groupWord && `${providerName} ${entry.groupWord} · `}
                        No resources
                      </span>
                    </td>
                    <td className={styles.whereCol}>
                      <span className={styles.whereRow}>
                        <span className={styles.nothing}>Nothing to import yet</span>
                        {project.url && (
                          <RowMenu
                            label={`More for ${project.name}`}
                            items={[
                              {
                                label: `Open on ${providerName}`,
                                onSelect: () => {
                                  void openExternal(project.url ?? '');
                                },
                              },
                            ]}
                          />
                        )}
                      </span>
                    </td>
                  </tr>
                </tbody>
              ))}
        </table>
        {groups.length === 0 && empty.length === 0 && (
          <p className={styles.none}>
            {words || type || status
              ? 'Nothing matches.'
              : everyAccount
                ? 'Nothing on your providers yet.'
                : `Nothing on this ${providerName} account yet.`}
          </p>
        )}
      </div>
      {moving && (
        <MoveDialog
          resource={moving.resource}
          current={picker.rowTarget(moving.project, moving.resource)}
          existing={picker.existing}
          onClose={() => {
            setMoving(null);
          }}
          onMove={(target) => {
            picker.moveResource(moving.resource.id, target);
            setMoving(null);
          }}
        />
      )}
    </div>
  );
}

/** Moving one resource: to a Hoista project there is, or a new one by name. */
function MoveDialog({
  resource,
  current,
  existing,
  onClose,
  onMove,
}: {
  resource: ProviderProjectResource;
  current: Target;
  existing: { id: string; name: string }[];
  onClose: () => void;
  onMove: (target: Target) => void;
}) {
  const [choice, setChoice] = useState<string | null>(null);
  const [name, setName] = useState(resource.name);
  const options: Option<string | null>[] = [
    { value: null, label: 'None' },
    ...existing
      .filter((item) => current.kind !== 'existing' || item.id !== current.id)
      .map((item) => ({ value: `existing:${item.id}`, label: item.name })),
    { value: 'new', label: 'A new project' },
  ];
  let target: Target | null = null;
  if (choice === 'new' && name.trim()) target = { kind: 'new', name: name.trim() };
  else if (choice?.startsWith('existing:')) {
    const id = choice.slice('existing:'.length);
    target = { kind: 'existing', id, name: existing.find((item) => item.id === id)?.name ?? id };
  }
  return (
    <Dialog
      title={`Move ${resource.name}`}
      eyebrow={`Now: ${current.kind === 'existing' ? current.name : `${current.name} (new)`}`}
      onClose={onClose}
      actions={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="accent"
            disabled={!target}
            onClick={() => {
              if (target) onMove(target);
            }}
          >
            Move
          </Button>
        </>
      }
    >
      <div className={styles.moveFields}>
        <Select large wide label="Project" value={choice} options={options} onChange={setChoice} />
        {choice === 'new' && (
          <label className={styles.moveName}>
            <span>New project name</span>
            <input
              value={name}
              onChange={(event) => {
                setName(event.target.value);
              }}
            />
          </label>
        )}
        <p className={styles.moveNote}>
          Only Hoista’s grouping changes. Nothing moves at the provider.
        </p>
      </div>
    </Dialog>
  );
}
