import { useId, useState } from 'react';
import { useAddDomain } from '@ui/api/projects';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { cx } from '@ui/lib/cx';
import styles from './AddDomainDialog.module.css';
import shared from './domains.module.css';
import { useProjectNames } from '@ui/lib/useProjectNames';

export function AddDomainDialog({
  projectId,
  onClose,
}: {
  projectId: string;
  onClose: () => void;
}) {
  const inputId = useId();
  const [name, setName] = useState('');
  const add = useAddDomain(projectId);
  const projectName = useProjectNames();

  return (
    <Dialog
      title="Add a domain"
      eyebrow={`${projectName(projectId)} · Domains`}
      onClose={onClose}
      actions={
        <>
          <Button large onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="accent"
            large
            disabled={!name || add.isPending}
            onClick={() => {
              add.mutate(name, { onSuccess: onClose });
            }}
          >
            Add domain
          </Button>
        </>
      }
    >
      <label htmlFor={inputId} className={styles.label}>
        Domain
      </label>
      <input
        id={inputId}
        className={cx('mono', shared.input)}
        placeholder="boards.example.com"
        autoComplete="off"
        spellCheck={false}
        value={name}
        onChange={(event) => {
          setName(event.target.value);
        }}
      />
      <p className={shared.cardText}>
        Hoista then shows the one DNS change to make and turns on HTTPS when it works.
      </p>
      {add.error && <ErrorMessage error={add.error} />}
    </Dialog>
  );
}
