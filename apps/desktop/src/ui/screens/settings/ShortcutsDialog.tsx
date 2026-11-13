/**
 * Keyboard shortcuts, as a dialog (⌘/, or Help → Keyboard Shortcuts): every
 * shortcut Hoista answers, grouped, from the one list the app menu uses.
 */
import { SHORTCUTS, type Shortcut } from '@shared/shortcuts';
import { Dialog } from '@ui/components/Dialog';
import { shortcutLabel } from '@ui/lib/keys';
import styles from './ShortcutsDialog.module.css';

/** Two columns: General and View on the left, Go to on the right. */
const COLUMNS: Shortcut['group'][][] = [['General', 'View'], ['Go to']];

export function ShortcutsDialog({ onClose }: { onClose: () => void }) {
  return (
    <Dialog title="Keyboard shortcuts" onClose={onClose}>
      <div className={styles.groups}>
        {COLUMNS.map((column) => (
          <div key={column.join()} className={styles.column}>
            {column.map((group) => (
              <section key={group} className={styles.group}>
                <h3 className={styles.title}>{group}</h3>
                <dl className={styles.list}>
                  {SHORTCUTS.filter((shortcut) => shortcut.group === group).map((shortcut) => (
                    <div key={shortcut.command} className={styles.row}>
                      <dt>{shortcut.label}</dt>
                      <dd>
                        <kbd className={styles.key}>{shortcutLabel(shortcut.key)}</kbd>
                      </dd>
                    </div>
                  ))}
                  {group === 'General' && (
                    <div className={styles.row}>
                      <dt>Close a dialog or menu</dt>
                      <dd>
                        <kbd className={styles.key}>Esc</kbd>
                      </dd>
                    </div>
                  )}
                </dl>
              </section>
            ))}
          </div>
        ))}
      </div>
    </Dialog>
  );
}
