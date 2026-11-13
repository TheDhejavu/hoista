/**
 * Moving to a new computer, on the new one: drop (or choose) the file saved on
 * the old computer, type its passphrase, and everything comes back. The same
 * window then says what arrived. Opens from the welcome screen, first-run
 * setup and Settings.
 */
import { useState, type DragEvent } from 'react';
import { Eye, MessageSquare, ShieldCheck, X } from 'lucide-react';
import { useImportTransfer } from '@ui/api/settings';
import { chooseTransferFile, pathForFile } from '@ui/api/system';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { SecretField } from '@ui/components/Field';
import { Mascot } from '@ui/components/Mascot';
import { cx } from '@ui/lib/cx';
import { plural } from '@ui/lib/format';
import styles from './ImportTransferDialog.module.css';

/** What a transfer file brings, shown before unlocking it. */
const BRINGS = [
  { title: 'Projects', text: 'With their resources and maps' },
  { title: 'Accounts', text: 'Tokens go into this computer’s keychain' },
  { title: 'Settings', text: 'Your name and preferences' },
];

const EXTENSION = '.hoista';

export function ImportTransferDialog({
  onClose,
  onImported,
}: {
  onClose: () => void;
  /** Called after the person has seen what came back and pressed Open Hoista. */
  onImported: () => void;
}) {
  const importTransfer = useImportTransfer();
  const [path, setPath] = useState<string | null>(null);
  const [passphrase, setPassphrase] = useState('');
  const [dragging, setDragging] = useState(false);
  const [wrongFile, setWrongFile] = useState<string | null>(null);
  const [choosing, setChoosing] = useState(false);
  const result = importTransfer.data;

  function take(chosen: string): void {
    if (!chosen.toLowerCase().endsWith(EXTENSION)) {
      setWrongFile(`${baseName(chosen)} isn’t a Hoista file. Use the ${EXTENSION} file you saved.`);
      return;
    }
    setWrongFile(null);
    setPath(chosen);
  }

  function choose(): void {
    setChoosing(true);
    void chooseTransferFile()
      .then((chosen) => {
        if (chosen) take(chosen);
      })
      .finally(() => {
        setChoosing(false);
      });
  }

  function onDrop(event: DragEvent): void {
    event.preventDefault();
    setDragging(false);
    const file = event.dataTransfer.files[0];
    if (!file) return;
    const dropped = pathForFile(file);
    if (dropped) take(dropped);
    else setWrongFile(`Hoista can’t open ${file.name} from there. Use Choose file instead.`);
  }

  if (result) {
    return (
      <Dialog title="Welcome back" plain onClose={onImported}>
        <div className={styles.welcome}>
          <span className={styles.mascot}>
            <Mascot mood="idle" size={64} />
          </span>
          <h2 id="dialog-title" className={styles.welcomeTitle}>
            {result.profile_name ? `Welcome back, ${result.profile_name}` : 'Welcome back'}
          </h2>
          <span className={styles.welcomeSub}>Everything is on this computer now</span>
          <div className={styles.counts}>
            <span className={styles.count}>
              <b>{plural(result.projects, 'project')}</b>
              <span>With their resources and maps</span>
            </span>
            <span className={styles.count}>
              <b>{plural(result.accounts, 'account')}</b>
              <span>Tokens saved in this computer’s keychain</span>
            </span>
          </div>
          <ul className={styles.notes}>
            <li>
              <Eye size={15} aria-hidden="true" />
              Every project arrives watched, so nothing changes at your providers until you say.
              Turn managing back on in each project’s Settings once you’ve checked it.
            </li>
            <li>
              <MessageSquare size={15} aria-hidden="true" />
              Agents connect once more: their access stays with the Hoista they joined. Settings →
              Agents has the address.
            </li>
          </ul>
        </div>
        <footer className={cx(styles.footer, styles.footerEnd)}>
          <Button variant="accent" large onClick={onImported}>
            Open Hoista
          </Button>
        </footer>
      </Dialog>
    );
  }

  return (
    <Dialog title="Bring Hoista over" plain onClose={onClose}>
      <div className={styles.main}>
        <header className={styles.head}>
          <span className={styles.headIcon} aria-hidden="true">
            <svg
              width="22"
              height="22"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.7"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <rect x="2" y="4" width="9" height="7" rx="1.5" />
              <path d="M1 14h11" />
              <rect x="13" y="10" width="9" height="7" rx="1.5" />
              <path d="M12 20h11M7 15v2a2 2 0 0 0 2 2h1M17 9V7a2 2 0 0 0-2-2h-1" />
            </svg>
          </span>
          <div className={styles.headText}>
            <h2 id="dialog-title">Bring Hoista over</h2>
            <span>From the file you saved on your old computer</span>
          </div>
          <button type="button" className={styles.close} aria-label="Close" onClick={onClose}>
            <X size={13} strokeWidth={2.4} aria-hidden="true" />
          </button>
        </header>

        <div
          className={cx(styles.drop, dragging && styles.dragging)}
          onDragOver={(event) => {
            if (!event.dataTransfer.types.includes('Files')) return;
            event.preventDefault();
            event.dataTransfer.dropEffect = 'copy';
            setDragging(true);
          }}
          onDragLeave={(event) => {
            if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
            setDragging(false);
          }}
          onDrop={onDrop}
        >
          <FileArt />
          {path ? (
            <>
              <span className={styles.dropTitle}>{baseName(path)}</span>
              <span className={cx(styles.dropText, styles.path)} title={path}>
                {path}
              </span>
            </>
          ) : (
            <>
              <span className={styles.dropTitle}>
                {dragging ? 'Drop it to use it' : 'Drop your Hoista file here'}
              </span>
              <span className={styles.dropText}>
                The locked file you saved with Settings → General → Move to a new computer.
              </span>
            </>
          )}
          <span className={styles.dropAction}>
            <Button busy={choosing} onClick={choose}>
              {path ? 'Choose another file' : 'Choose file'}
            </Button>
          </span>
        </div>
        {wrongFile && (
          <p className={styles.wrong} role="alert">
            {wrongFile}
          </p>
        )}

        <SecretField
          label="Passphrase"
          placeholder="The one you picked when you saved it"
          value={passphrase}
          onChange={setPassphrase}
          revealable
        />

        <ul className={styles.brings} aria-label="What comes over">
          {BRINGS.map((item) => (
            <li key={item.title}>
              <b>{item.title}</b>
              <span>{item.text}</span>
            </li>
          ))}
        </ul>
        <ErrorMessage error={importTransfer.error} />
      </div>
      <footer className={styles.footer}>
        <span className={styles.footNote}>
          <ShieldCheck size={14} aria-hidden="true" />
          Nothing changes at your providers
        </span>
        <span className={styles.footActions}>
          <Button variant="ghost" large onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="accent"
            large
            busy={importTransfer.isPending}
            busyLabel="Unlocking"
            disabled={!path || !passphrase}
            onClick={() => {
              if (path) importTransfer.mutate({ path, passphrase });
            }}
          >
            Unlock and restore
          </Button>
        </span>
      </footer>
    </Dialog>
  );
}

/** The last part of a path: `sam.hoista` from `~/Downloads/sam.hoista`. */
function baseName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

/** A locked file, the picture in the drop zone. */
function FileArt() {
  return (
    <svg className={styles.fileArt} width="40" height="48" viewBox="0 0 40 48" aria-hidden="true">
      <path
        d="M4 2h22l10 10v32a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2z"
        className={styles.paper}
        strokeWidth="1.5"
      />
      <path d="M26 2v8a2 2 0 0 0 2 2h8" fill="none" className={styles.fold} strokeWidth="1.5" />
      <rect x="12" y="24" width="16" height="12" rx="2.5" className={styles.lock} />
      <path
        d="M15 24v-3a5 5 0 0 1 10 0v3"
        fill="none"
        className={styles.shackle}
        strokeWidth="2.2"
      />
    </svg>
  );
}
