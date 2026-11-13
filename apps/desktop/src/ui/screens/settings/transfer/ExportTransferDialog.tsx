/**
 * Moving to a new computer, on the old one: pick a passphrase and Hoista
 * saves everything to one encrypted file. Secret values are not in it; they
 * live at each provider and never come back to Hoista.
 */
import { useState } from 'react';
import { useExportTransfer } from '@ui/api/settings';
import { chooseTransferSave } from '@ui/api/system';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { SecretField } from '@ui/components/Field';
import styles from './ExportTransferDialog.module.css';

export function ExportTransferDialog({ onClose }: { onClose: () => void }) {
  const exportTransfer = useExportTransfer();
  const [passphrase, setPassphrase] = useState('');
  const [again, setAgain] = useState('');
  const mismatch = again.length > 0 && again !== passphrase;
  const file = exportTransfer.data;

  if (file) {
    return (
      <Dialog
        title="Ready to move"
        eyebrow={`Saved to ${file.path}`}
        onClose={onClose}
        actions={
          <Button variant="primary" onClick={onClose}>
            Done
          </Button>
        }
      >
        <div className={styles.body}>
          <ul className={styles.list}>
            {file.contents.map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ul>
          <ol className={styles.steps}>
            <li>Copy the file to your new computer: AirDrop, a USB stick or your cloud drive.</li>
            <li>Install Hoista there and choose “Moving from another computer?”.</li>
            <li>
              Type the same passphrase. Your projects, accounts and maps come back, watched until
              you turn managing on again.
            </li>
          </ol>
        </div>
      </Dialog>
    );
  }

  return (
    <Dialog
      title="Move to a new computer"
      eyebrow="Saves everything to one encrypted file"
      onClose={onClose}
      actions={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={!passphrase || passphrase !== again || exportTransfer.isPending}
            onClick={() => {
              void chooseTransferSave().then((path) => {
                if (path) exportTransfer.mutate({ path, passphrase });
              });
            }}
          >
            {exportTransfer.isPending ? 'Saving' : 'Save file'}
          </Button>
        </>
      }
    >
      <div className={styles.body}>
        <div className={styles.columns}>
          <div>
            <h3 className={styles.heading}>Goes in the file</h3>
            <ul className={styles.list}>
              <li>Your profile and settings</li>
              <li>Every project, its resources and how you arranged its map</li>
              <li>Account tokens, from your keychain</li>
              <li>Agent activity</li>
            </ul>
          </div>
          <div>
            <h3 className={styles.heading}>Stays behind</h3>
            <ul className={styles.list}>
              <li>Project secrets: they live at each provider, so nothing to move</li>
              <li>Your remote: it keeps running in your account, with the same URL</li>
              <li>Agents’ access: each agent connects once more on the new computer</li>
            </ul>
          </div>
        </div>
        <SecretField
          label="Passphrase"
          placeholder="At least 12 characters"
          value={passphrase}
          onChange={setPassphrase}
          hint="The file holds your account tokens, so it's locked with this. Hoista can't recover it if you forget."
        />
        <SecretField
          label="Type it again"
          placeholder="Same passphrase"
          value={again}
          onChange={setAgain}
          hint={mismatch ? 'These don’t match yet.' : undefined}
        />
        <ErrorMessage error={exportTransfer.error} />
      </div>
    </Dialog>
  );
}
