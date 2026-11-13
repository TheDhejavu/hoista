/**
 * Which Hoista environment the window shows, under the search box, like a
 * team switcher: pick one, add one, or manage them. The Projects list, recent
 * projects and imports follow it.
 */
import { Layers } from 'lucide-react';
import { useState } from 'react';
import { useCurrentEnvironment } from '@ui/api/environments';
import { PillSelect, type Option } from '@ui/components/Select';
import { Skeleton } from '@ui/components/Skeleton';
import { EnvironmentsDialog, NameDialog } from '@ui/screens/environments/EnvironmentsDialog';
import { showToast } from '@ui/lib/toast';
import styles from './Sidebar.module.css';

const NEW = '__new';
const MANAGE = '__manage';

export function EnvironmentSwitcher() {
  const current = useCurrentEnvironment();
  const [open, setOpen] = useState<'new' | 'manage' | null>(null);
  // There's always one, so none means still loading: keep its place.
  if (current.environments.length === 0) {
    return (
      <div className={styles.environment} aria-hidden="true">
        <Skeleton height={30} width="100%" later className={styles.environmentSkeleton} />
      </div>
    );
  }

  const options: Option<string>[] = [
    ...current.environments.map((environment) => ({
      value: environment.id,
      label: environment.name,
      group: 'Environments',
    })),
    { value: NEW, label: 'New environment', group: 'Manage' },
    { value: MANAGE, label: 'Manage environments', group: 'Manage' },
  ];

  return (
    <div className={styles.environment}>
      <PillSelect
        label="Environment"
        icon={<Layers size={13} aria-hidden="true" />}
        value={current.id}
        options={options}
        onChange={(value) => {
          if (value === NEW) setOpen('new');
          else if (value === MANAGE) setOpen('manage');
          else {
            current.set(value);
            const name = current.environments.find((item) => item.id === value)?.name ?? value;
            showToast(`Showing ${name}`);
          }
        }}
      />
      {open === 'manage' && (
        <EnvironmentsDialog
          onClose={() => {
            setOpen(null);
          }}
        />
      )}
      {open === 'new' && (
        <NameDialog
          environment={null}
          onCreated={(made) => {
            current.set(made.id);
          }}
          onClose={() => {
            setOpen(null);
          }}
        />
      )}
    </div>
  );
}
