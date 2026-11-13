/**
 * Settings for a project Hoista watches, in the same cards as a managed project's:
 * where its code comes from (the repository its provider deploys, or a folder),
 * whether the provider
 * lets it sleep when idle, its name, the resources it holds, and stopping
 * watching it. Every control here does what it says: the code link,
 * name and resources are Hoista's own; sleep is changed at the provider.
 */
import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import type { ProjectDetail, Resource } from '@shared/api';
import { serviceName } from '@shared/providers';
import { useProviderDetails, useLinkSource, useStopWatching } from '@ui/api/projects';
import { useCan } from '@ui/api/providers';
import { useRenameProject, useSplitResource } from '@ui/api/imports';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { ExternalLink } from '@ui/components/ExternalLink';
import { commitUrl, repoUrl, splitSource } from '@ui/lib/links';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { resourceKindLabel, runsCode } from '../resourceKinds';
import { RemoveFromProject } from '../RemoveFromProject';
import { DeleteAtProvider } from '../DeleteAtProvider';
import { Card } from './SettingsTab';
import { EnvironmentCard } from './EnvironmentCard';
import { RowMenu } from '@ui/components/RowMenu';
import { openExternal } from '@ui/api/system';
import { CurrentSource } from './SourcePicker';
import { ChangeSourceDialog } from './ChangeSourceDialog';
import styles from './SettingsTab.module.css';

/** A name the engine accepts, from a provider's display name. */
function newProjectName(display: string): string {
  return (
    display
      .normalize('NFKD')
      .replace(/[̀-ͯ]/g, '')
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .slice(0, 63) || 'project'
  );
}

export function WatchedSettings({ detail }: { detail: ProjectDetail }) {
  // One provider by name; a project on several says "your providers".
  const providers = [...new Set(detail.resources.map((resource) => resource.provider))];
  const provider =
    providers.length === 1 && providers[0] ? serviceName(providers[0]) : 'your providers';
  // The resource that runs code: its provider deployments say which repository it builds.
  const services = detail.resources.filter((resource) => runsCode(resource.kind));
  const codeResource = services[0] ?? null;
  const codeProvider = codeResource ? serviceName(codeResource.provider) : provider;

  return (
    <>
      <CodeCard detail={detail} provider={codeProvider} services={services} />
      <NameCard detail={detail} provider={provider} />
      <EnvironmentCard detail={detail} />
      <ResourcesCard detail={detail} provider={provider} />
      <StopWatchingCard detail={detail} provider={provider} />
    </>
  );
}

/**
 * The project's source, as providers show it: the connected repository or folder
 * as one row with its ⋮, or a button to connect one. Connecting and
 * changing happen in steps (ChangeSourceDialog), so nothing changes before
 * the person has seen what will.
 */
export function CodeCard({
  detail,
  provider,
  services,
  managed = false,
}: {
  detail: ProjectDetail;
  provider: string;
  /** Everything in the app that runs code: a frontend, an API, a worker. */
  services: Resource[];
  /** Hoista builds it: the source is where every deploy comes from. */
  managed?: boolean;
}) {
  const { project } = detail;
  const codeResource = services[0] ?? null;
  const several = services.length > 1;
  const link = useLinkSource(project.id);
  const [dialog, setDialog] = useState<'change' | 'disconnect' | null>(null);
  // The service the change is for, when picked from its row.
  const [forService, setForService] = useState<Resource | null>(null);
  const latest = useProviderDetails(codeResource?.id ?? null).data?.deployments.find(
    (deployment) => deployment.repo && deployment.commit,
  );
  const deployed =
    latest?.repo && latest.commit ? { repo: latest.repo, commit: latest.commit } : null;
  const linkedRepo = project.source.kind === 'github' ? project.source.ref.split('@')[0] : null;
  const linkedFolder =
    project.source.kind === 'folder' && project.source.ref ? project.source.ref : null;
  const connected = linkedRepo !== null || linkedFolder !== null;
  // Hoista points at one repository while the provider builds another.
  const mismatch = linkedRepo && deployed && deployed.repo !== linkedRepo ? deployed : null;
  const repo = linkedRepo ? splitSource(project.source.ref) : null;
  const text = managed
    ? linkedRepo
      ? 'Hoista builds each deploy from this GitHub repository.'
      : linkedFolder
        ? 'Hoista builds each deploy from this folder on this computer.'
        : 'Connect the folder or GitHub repository Hoista builds this project from.'
    : linkedRepo
      ? 'Connected to a GitHub repository. Agents push here, and Deploys builds its commits.'
      : linkedFolder
        ? 'Connected to a folder on this computer. You and your agents work on the code there.'
        : 'Connect the Git repository this project deploys from, or the folder you work in, so you and your agents change the right code.';

  return (
    <Card
      title="Source"
      text={text}
      footer={
        managed
          ? 'The next deploy uses the new source.'
          : several
            ? 'Each service builds from its own repository at its provider. Agents work in the source above.'
            : deployed
              ? `${provider} deploys ${deployed.repo}.`
              : `${provider} hasn’t reported which repository it deploys.`
      }
    >
      {connected ? (
        <CurrentSource
          kind={linkedRepo ? 'github' : 'folder'}
          name={linkedRepo ?? linkedFolder ?? ''}
          url={repo ? repoUrl(repo.repo) : null}
          meta={
            repo ? (
              <>
                <span>GitHub</span>
                {commitUrl(repo.repo, repo.commit) && (
                  <ExternalLink variant="text" url={commitUrl(repo.repo, repo.commit) ?? ''}>
                    Commit {repo.commit}
                  </ExternalLink>
                )}
              </>
            ) : project.source.git ? (
              // A folder that is a clone: where it pushes, as on its provider.
              <>
                <span>Local folder</span>
                <ExternalLink variant="text" url={repoUrl(project.source.git.repository) ?? ''}>
                  GitHub · {project.source.git.repository}
                  {project.source.git.branch ? ` · ${project.source.git.branch}` : ''}
                </ExternalLink>
                {project.source.git.commit &&
                  commitUrl(project.source.git.repository, project.source.git.commit) && (
                    <ExternalLink
                      variant="text"
                      url={
                        commitUrl(project.source.git.repository, project.source.git.commit) ?? ''
                      }
                    >
                      Commit {project.source.git.commit.slice(0, 7)}
                    </ExternalLink>
                  )}
              </>
            ) : (
              <span>Local folder</span>
            )
          }
          menu={
            <RowMenu
              label="More for the source"
              items={[
                ...(!repo && project.source.git && repoUrl(project.source.git.repository)
                  ? [
                      {
                        label: 'Open on GitHub',
                        onSelect: () => {
                          void openExternal(repoUrl(project.source.git?.repository ?? '') ?? '');
                        },
                      },
                    ]
                  : []),
                ...(repo && repoUrl(repo.repo)
                  ? [
                      {
                        label: 'Open on GitHub',
                        onSelect: () => {
                          void openExternal(repoUrl(repo.repo) ?? '');
                        },
                      },
                    ]
                  : []),
                {
                  label: 'Change source',
                  onSelect: () => {
                    setForService(null);
                    setDialog('change');
                  },
                },
                ...(managed
                  ? []
                  : [
                      {
                        label: 'Disconnect',
                        danger: true,
                        onSelect: () => {
                          setDialog('disconnect');
                        },
                      },
                    ]),
              ]}
            />
          }
        />
      ) : (
        <Button
          variant="accent"
          onClick={() => {
            setDialog('change');
          }}
        >
          Connect source
        </Button>
      )}
      {several && (
        <div className={styles.services}>
          <span className={styles.servicesTitle}>What each service builds from</span>
          {services.map((service) => (
            <ServiceSource
              key={service.id}
              service={service}
              onChange={() => {
                setForService(service);
                setDialog('change');
              }}
            />
          ))}
        </div>
      )}
      {!managed && !several && mismatch && (
        <div className={styles.mismatch} role="status">
          <span>
            <b>
              {provider} deploys {mismatch.repo}
            </b>
            , not {linkedRepo}. Agents would push to a repository that isn’t live.
          </span>
          <Button
            busy={link.isPending}
            busyLabel="Connecting"
            onClick={() => {
              link.mutate({ kind: 'github', repository: mismatch.repo, commit: mismatch.commit });
            }}
          >
            Use {mismatch.repo}
          </Button>
        </div>
      )}
      <ErrorMessage error={link.error} />
      {dialog === 'change' && (
        <ChangeSourceDialog
          detail={detail}
          services={services}
          initial={forService}
          onClose={() => {
            setDialog(null);
          }}
        />
      )}
      {dialog === 'disconnect' && (
        <Dialog
          title="Disconnect the source?"
          eyebrow={linkedRepo ?? linkedFolder ?? ''}
          onClose={() => {
            setDialog(null);
          }}
          actions={
            <>
              <Button
                variant="ghost"
                onClick={() => {
                  setDialog(null);
                }}
              >
                Keep it
              </Button>
              <Button
                variant="danger"
                busy={link.isPending}
                busyLabel="Disconnecting"
                onClick={() => {
                  link.mutate(
                    { kind: 'unlinked' },
                    {
                      onSuccess: () => {
                        setDialog(null);
                      },
                    },
                  );
                }}
              >
                Disconnect
              </Button>
            </>
          }
        >
          <p className={styles.muted}>
            Hoista and your agents stop using {linkedRepo ?? 'this folder'}. Nothing changes at{' '}
            {provider}: it keeps deploying what it deploys now
            {deployed ? `, ${deployed.repo}` : ''}.
          </p>
          <ErrorMessage error={link.error} />
        </Dialog>
      )}
    </Card>
  );
}

/** One service's row: what its provider says it builds from, and its ⋮. */
function ServiceSource({ service, onChange }: { service: Resource; onChange: () => void }) {
  const latest = useProviderDetails(service.id).data?.deployments.find(
    (deployment) => deployment.repo && deployment.commit,
  );
  const repo = latest?.repo ?? null;
  return (
    <div className={styles.serviceSource}>
      <ServiceIcon id={service.provider} size={16} />
      <span className={styles.serviceSourceText}>
        <b>{service.name}</b>
        <span className={styles.muted}>
          {resourceKindLabel(service.kind)} on {serviceName(service.provider)}
        </span>
      </span>
      <span className={styles.serviceRepo}>
        {repo ? (
          <ExternalLink variant="text" url={repoUrl(repo) ?? ''}>
            <ServiceIcon id="github" size={13} />
            {repo}
            {latest?.commit ? ` · ${latest.commit.slice(0, 7)}` : ''}
          </ExternalLink>
        ) : (
          <span className={styles.muted}>No repository reported</span>
        )}
      </span>
      <RowMenu
        label={`More for ${service.name}`}
        items={[{ label: 'Change source', onSelect: onChange }]}
      />
    </div>
  );
}

export function NameCard({ detail, provider }: { detail: ProjectDetail; provider: string }) {
  const { project } = detail;
  const [name, setName] = useState(project.name);
  const rename = useRenameProject(project.id);

  return (
    <Card
      title="Name"
      text={`What Hoista calls this project. Names on ${provider} stay as they are.`}
      footer={
        <>
          <span>Agents find it by this name too.</span>
          <Button
            variant="accent"
            large
            disabled={name.trim() === project.name || !name.trim() || rename.isPending}
            onClick={() => {
              rename.mutate(newProjectName(name));
            }}
          >
            {rename.isPending ? 'Saving' : 'Save'}
          </Button>
        </>
      }
    >
      <input
        aria-label="Project name"
        className={styles.inlineInput}
        value={name}
        onChange={(event) => {
          setName(event.target.value);
        }}
      />
      <ErrorMessage error={rename.error} />
    </Card>
  );
}

function ResourcesCard({ detail, provider }: { detail: ProjectDetail; provider: string }) {
  const { resources } = detail;

  return (
    <section className={styles.card}>
      <div className={styles.head}>
        <h2>Resources in this project</h2>
        <p>Hoista groups these together. Nothing moves on {provider}.</p>
      </div>
      <ul className={styles.resources}>
        {resources.map((resource) => (
          <li key={resource.id} className={styles.resource}>
            <ServiceIcon id={resource.provider} size={16} />
            <span>{resource.name}</span>
            <span className={styles.muted}>
              {resourceKindLabel(resource.kind)} on {serviceName(resource.provider)}
            </span>
            <span className={styles.resourceActions}>
              <ResourceMenu
                resource={resource}
                projectName={detail.project.name}
                count={resources.length}
              />
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** A resource row's "⋮": its own project, another project, out of this one, or deleted at the provider. */
function ResourceMenu({
  resource,
  projectName,
  count,
}: {
  resource: ProjectDetail['resources'][number];
  projectName: string;
  count: number;
}) {
  const navigate = useNavigate();
  const split = useSplitResource();
  const can = useCan();
  const [dialog, setDialog] = useState<'remove' | 'delete' | null>(null);
  const provider = serviceName(resource.provider);
  return (
    <>
      <RowMenu
        label={`More for ${resource.name}`}
        items={[
          ...(count > 1 && runsCode(resource.kind)
            ? [
                {
                  label: split.isPending ? 'Making its own project' : 'Make it its own project',
                  disabled: split.isPending,
                  onSelect: () => {
                    split.mutate(
                      { resourceId: resource.id, name: newProjectName(resource.name) },
                      {
                        onSuccess: (created) => {
                          void navigate(`/projects/${created.id}`);
                        },
                      },
                    );
                  },
                },
              ]
            : []),
          {
            label: 'Move to another project',
            onSelect: () => {
              void navigate(`/projects/${resource.project_id}/resources/${resource.id}/settings`);
            },
          },
          {
            label: 'Remove from project',
            onSelect: () => {
              setDialog('remove');
            },
          },
          ...(can(resource.provider, 'delete')
            ? [
                {
                  label: `Delete on ${provider}`,
                  danger: true,
                  onSelect: () => {
                    setDialog('delete');
                  },
                },
              ]
            : []),
        ]}
      />
      <RemoveFromProject
        resource={resource}
        projectName={projectName}
        last={count === 1}
        opened={dialog === 'remove'}
        onClosed={() => {
          setDialog(null);
        }}
      />
      <DeleteAtProvider
        resource={resource}
        last={count === 1}
        opened={dialog === 'delete'}
        onClosed={() => {
          setDialog(null);
        }}
      />
      <ErrorMessage error={split.error} />
    </>
  );
}

/** Stop watching: Hoista forgets the project; nothing changes at its providers. */
function StopWatchingCard({ detail, provider }: { detail: ProjectDetail; provider: string }) {
  const { project } = detail;
  const [open, setOpen] = useState(false);
  const stop = useStopWatching(project.id);
  const navigate = useNavigate();

  return (
    <>
      <Card
        danger
        title={`Stop watching ${project.name}`}
        text={`Hoista forgets this project. It keeps running on ${provider}, and its resources go back under Found on your providers.`}
        footer={
          <>
            <span>Nothing is deleted on {provider}.</span>
            <Button
              variant="danger"
              large
              onClick={() => {
                setOpen(true);
              }}
            >
              Stop watching
            </Button>
          </>
        }
      />
      {open && (
        <Dialog
          title={`Stop watching ${project.name}?`}
          onClose={() => {
            setOpen(false);
          }}
          actions={
            <>
              <Button
                large
                onClick={() => {
                  setOpen(false);
                }}
              >
                Cancel
              </Button>
              <Button
                variant="danger"
                large
                disabled={stop.isPending}
                onClick={() => {
                  stop.mutate(undefined, {
                    onSuccess: () => {
                      void navigate('/');
                    },
                  });
                }}
              >
                Stop watching
              </Button>
            </>
          }
        >
          <p className={styles.muted}>
            {project.name} keeps running on {provider}. You can add it again from Found on your
            providers.
          </p>
          <ErrorMessage error={stop.error} />
        </Dialog>
      )}
    </>
  );
}
