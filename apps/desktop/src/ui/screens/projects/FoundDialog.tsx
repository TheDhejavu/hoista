/**
 * Add projects from what connecting your providers found, laid out the way the provider
 * organises them: each connected account (a Railway workspace) and, inside
 * it, each provider project. A project becomes one project with the resources you
 * leave ticked, under a name you can change. Nothing changes at the provider.
 */
import { useMemo } from 'react';
import type { FoundOnProviders, FoundResource, ProviderId } from '@shared/api';
import { serviceName } from '@shared/providers';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { showToast } from '@ui/lib/toast';
import { ProjectPicker } from './ProjectPicker';
import { projectsFromFound, useApplyPicker, usePicker } from './usePicker';

/** One proposed project: a provider project, or a single resource with no project. */
interface Proposal {
  key: string;
  name: string;
  resources: FoundResource[];
  /** A project already holding this project: its resources go there instead. */
  join: { project_id: string; project_name: string } | null;
}

interface AccountSection {
  id: string;
  provider: ProviderId;
  label: string;
  proposals: Proposal[];
}

export function sections(found: FoundOnProviders): AccountSection[] {
  const byAccount = new Map<string, AccountSection>();
  for (const resource of found.resources) {
    let section = byAccount.get(resource.account.id);
    if (!section) {
      section = {
        id: resource.account.id,
        provider: resource.provider,
        label: resource.account.label,
        proposals: [],
      };
      byAccount.set(resource.account.id, section);
    }
    const key = resource.group ? `${resource.account.id}:${resource.group}` : resource.id;
    let proposal = section.proposals.find((item) => item.key === key);
    if (!proposal) {
      proposal = { key, name: resource.group ?? resource.name, resources: [], join: resource.join };
      section.proposals.push(proposal);
    }
    proposal.resources.push(resource);
  }
  return [...byAccount.values()];
}

export function FoundDialog({
  found,
  onClose,
  onConnect,
}: {
  found: FoundOnProviders;
  onClose: () => void;
  /** Connect another account at this provider, such as a second workspace. */
  onConnect?: (provider: ProviderId) => void;
}) {
  const providers = useMemo(
    () => [...new Set(found.resources.map((item) => item.provider))],
    [found],
  );
  const provider = providers[0];
  const projects = useMemo(() => projectsFromFound(found, provider), [found, provider]);
  const picker = usePicker(projects, 'everything');
  const { apply, busy, error } = useApplyPicker();
  const count = picker.plan.create.length;
  const title =
    providers.length === 1 && providers[0]
      ? `Import from ${serviceName(providers[0])}`
      : 'Import projects';

  async function add(): Promise<void> {
    if (await apply(picker.plan)) {
      showToast(
        count === 0
          ? 'Added to your project'
          : count === 1
            ? 'Added 1 project'
            : `Added ${String(count)} projects`,
      );
      onClose();
    }
  }

  if (!provider) return null;
  return (
    <Dialog
      title={title}
      eyebrow="Each provider project is imported into a Hoista project of the same name. Change where it goes, untick what to leave out, or drag a resource to another project. Nothing changes at the provider."
      icon={<ServiceIcon id={provider} size={18} />}
      onClose={onClose}
      wide
      sideAction={
        onConnect ? (
          <Button
            variant="ghost"
            onClick={() => {
              onConnect(provider);
            }}
          >
            Connect another account
          </Button>
        ) : undefined
      }
      actions={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            busy={busy}
            busyLabel="Importing"
            disabled={!picker.changed}
            onClick={() => {
              void add();
            }}
          >
            Import
          </Button>
        </>
      }
    >
      <ProjectPicker picker={picker} provider={provider} />
      <ErrorMessage error={error} />
    </Dialog>
  );
}
