import { useId, useState } from 'react';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import styles from './domains.module.css';

/** One number: a port. */
export function PortDialog({
  title,
  text,
  onClose,
  onSave,
  busy,
  error,
}: {
  title: string;
  text: string;
  onClose: () => void;
  onSave: (port: number) => void;
  busy: boolean;
  error: Error | null;
}) {
  const [port, setPort] = useState('');
  const id = useId();
  const valid = /^\d+$/.test(port) && Number(port) > 0 && Number(port) < 65536;
  return (
    <Dialog
      title={title}
      onClose={onClose}
      actions={
        <>
          <Button large onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            large
            disabled={!valid || busy}
            onClick={() => {
              onSave(Number(port));
            }}
          >
            {busy ? 'Saving' : 'Save'}
          </Button>
        </>
      }
    >
      <p className={styles.dialogText}>{text}</p>
      <label htmlFor={id} className={styles.fieldLabel}>
        Port
      </label>
      <input
        id={id}
        className={styles.input}
        inputMode="numeric"
        placeholder="8080"
        value={port}
        onChange={(event) => {
          setPort(event.target.value.trim());
        }}
      />
      <ErrorMessage error={error} />
    </Dialog>
  );
}
