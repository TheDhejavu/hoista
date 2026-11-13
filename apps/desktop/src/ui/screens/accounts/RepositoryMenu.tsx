/**
 * A repository's "⋮": open it on GitHub, deploy it to a provider that builds
 * from GitHub, or copy a sentence that asks your agent to deploy it. Deploy
 * opens over the account window, which stays open underneath.
 */
import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import type { ProviderRepository } from '@engine-api';
import { serviceName } from '@shared/providers';
import { useAccounts } from '@ui/api/accounts';
import { useCan } from '@ui/api/providers';
import { useNewProjectAtProvider } from '@ui/api/repositories';
import { openExternal } from '@ui/api/system';
import { Button } from '@ui/components/Button';
import { Dialog, DialogText } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { TextField } from '@ui/components/Field';
import { RowMenu } from '@ui/components/RowMenu';
import { SelectField } from '@ui/components/Select';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { showToast } from '@ui/lib/toast';
import styles from './RepositoryMenu.module.css';

/** A repository's name as a project name: lowercase letters, digits and hyphens. */
function projectName(repository: string): string {
  const name = repository.split('/').at(-1) ?? repository;
  return (
    name
      .toLowerCase()
      .replace(/[^a-z0-9-]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .slice(0, 63) || 'project'
  );
}

export function RepositoryMenu({ repo }: { repo: ProviderRepository }) {
  const [deploying, setDeploying] = useState(false);
  const prompt = `Using Hoista, deploy the GitHub repository ${repo.full_name} (branch ${repo.default_branch}) to one of my connected providers, and tell me its address once it's live.`;
  return (
    <>
      <RowMenu
        label={`More for ${repo.full_name}`}
        items={[
          {
            label: 'Open on GitHub',
            onSelect: () => {
              void openExternal(repo.url);
            },
          },
          {
            label: 'Deploy to a provider',
            onSelect: () => {
              setDeploying(true);
            },
          },
          {
            label: 'Copy a prompt for your agent',
            onSelect: () => {
              void navigator.clipboard.writeText(prompt).then(() => {
                showToast('Copied. Paste it to your agent');
              });
            },
          },
        ]}
      />
      {deploying && (
        <DeployRepository
          repo={repo}
          onClose={() => {
            setDeploying(false);
          }}
        />
      )}
    </>
  );
}

/** What to ask an agent: the repository, branch, and the provider and name when chosen. */
function promptFor(
  repo: ProviderRepository,
  branch: string,
  provider: string | null,
  name: string,
) {
  return `Using Hoista, deploy the GitHub repository ${repo.full_name} (branch ${branch})${
    provider
      ? ` to my ${provider} account as a project named ${name}`
      : ' to one of my connected providers'
  }, and tell me its address once it's live.`;
}

/**
 * Deploying a repository: where to (a provider that builds from GitHub), what
 * to call the app, and which branch. Deploy here, or copy the same choices
 * as a prompt for your agent.
 */
function DeployRepository({ repo, onClose }: { repo: ProviderRepository; onClose: () => void }) {
  const can = useCan();
  const navigate = useNavigate();
  const create = useNewProjectAtProvider();
  const accounts = useAccounts();
  const providers = (accounts.data?.items ?? []).filter(
    (account) => account.status === 'ok' && can(account.service, 'new_project'),
  );
  const [accountId, setAccountId] = useState<string | null>(null);
  const [name, setName] = useState(() => projectName(repo.full_name));
  const [branch, setBranch] = useState(repo.default_branch);
  const chosen = providers.find((account) => account.id === accountId) ?? null;
  const provider = chosen ? serviceName(chosen.service) : null;
  const ready = Boolean(chosen && name.trim() && branch.trim());

  return (
    <Dialog
      title={`Deploy ${repo.full_name}`}
      icon={<ServiceIcon id="github" size={18} />}
      onClose={() => {
        if (!create.isPending) onClose();
      }}
      sideAction={
        <Button
          onClick={() => {
            void navigator.clipboard
              .writeText(
                promptFor(repo, branch.trim() || repo.default_branch, provider, name.trim()),
              )
              .then(() => {
                showToast('Copied. Paste it to your agent');
              });
          }}
        >
          Copy prompt
        </Button>
      }
      actions={
        <>
          <Button disabled={create.isPending} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="accent"
            disabled={!ready}
            busy={create.isPending}
            busyLabel="Creating"
            onClick={() => {
              if (!chosen) return;
              create.mutate(
                {
                  accountId: chosen.id,
                  name: name.trim(),
                  repository: repo.full_name,
                  branch: branch.trim(),
                },
                {
                  onSuccess: (made) => {
                    showToast(made.summary);
                    onClose();
                    void navigate(`/projects/${made.project.id}`);
                  },
                },
              );
            }}
          >
            {provider ? `Deploy on ${provider}` : 'Deploy'}
          </Button>
        </>
      }
    >
      <DialogText>
        The provider makes a new project, builds the branch, and gives it a public address. It
        deploys again each time you or your agent push to that branch.
      </DialogText>
      {accounts.isPending ? (
        <p role="status" className={styles.muted}>
          Looking at your providers
        </p>
      ) : providers.length === 0 ? (
        <DialogText>
          None of your connected providers can start a project from a repository yet. Connect one
          that builds from GitHub, or copy the prompt and ask your agent to deploy it.
        </DialogText>
      ) : (
        <div className={styles.fields}>
          <SelectField
            label="Deploy to"
            value={accountId}
            options={[
              { value: null, label: 'None' },
              ...providers.map((account) => ({
                value: account.id,
                label: serviceName(account.service),
                note: account.label,
              })),
            ]}
            onChange={setAccountId}
          />
          <TextField
            label="Project name"
            value={name}
            hint="Lowercase letters, digits and hyphens. The project at the provider gets this name."
            onChange={setName}
          />
          <TextField
            label="Branch"
            value={branch}
            hint={`The repository builds from ${repo.default_branch} by default.`}
            onChange={setBranch}
          />
          <div className={styles.summary}>
            <span>Repository</span>
            <span className="mono">{repo.full_name}</span>
            <span>Visibility</span>
            <span>{repo.private ? 'Private' : 'Public'}</span>
          </div>
        </div>
      )}
      <ErrorMessage error={create.error} />
    </Dialog>
  );
}
