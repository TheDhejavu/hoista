/**
 * The command palette (⌘K): type to find a project, a screen or an action,
 * move with the arrow keys and press Enter to go there.
 */
import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { useMatch, useNavigate } from 'react-router-dom';
import { openExternal } from '@ui/api/system';
import { showToast } from '@ui/lib/toast';
import { useProjects } from '@ui/api/projects';
import { ALL_ENVIRONMENTS } from '@ui/api/environments';
import { Search } from 'lucide-react';
import { useOverlays } from '@ui/layout/useOverlays';
import { cx } from '@ui/lib/cx';
import { shortcutLabel } from '@ui/lib/keys';
import { buildCommands, filterCommands, splitMatch, type Command } from './commands';
import styles from './CommandPalette.module.css';

export function CommandPalette({ onClose }: { onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const navigate = useNavigate();
  const { openSettings, openHelp, openShortcuts } = useOverlays();
  const projects = useProjects({ sort: 'name', environment: ALL_ENVIRONMENTS });
  const [search, setSearch] = useState('');
  const [selected, setSelected] = useState(0);

  useEffect(() => {
    dialog.current?.showModal();
  }, []);

  // Inside a project, its own commands come first.
  const current = useMatch('/projects/:projectId/*')?.params.projectId;
  const results = filterCommands(buildCommands(projects.data?.items ?? [], current), search);

  function run(command: Command | undefined): void {
    if (!command) return;
    onClose();
    if ('to' in command) void navigate(command.to);
    else if ('external' in command) void openExternal(command.external);
    else if ('copy' in command) {
      void navigator.clipboard.writeText(command.copy).then(
        () => {
          showToast(`${command.copied} copied`);
        },
        () => {
          showToast('Couldn’t copy it', { tone: 'error' });
        },
      );
    } else if (command.opens === 'settings') openSettings();
    else if (command.opens === 'shortcuts') openShortcuts();
    else openHelp();
  }

  function onKeyDown(event: KeyboardEvent<HTMLInputElement>): void {
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      setSelected((index) => Math.min(index + 1, results.length - 1));
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      setSelected((index) => Math.max(index - 1, 0));
    } else if (event.key === 'Enter') {
      event.preventDefault();
      run(results[selected]);
    }
  }

  return (
    <dialog ref={dialog} className={styles.dialog} aria-label="Command palette" onClose={onClose}>
      <div className={styles.search}>
        <Search size={15} />
        <input
          className={styles.input}
          placeholder="Search projects and commands"
          aria-label="Search commands"
          autoComplete="off"
          value={search}
          onKeyDown={onKeyDown}
          onChange={(event) => {
            setSearch(event.target.value);
            setSelected(0);
          }}
        />
        <kbd className={styles.key}>esc</kbd>
      </div>

      {results.length === 0 ? (
        <p className={styles.empty}>Nothing matches "{search}".</p>
      ) : (
        <ul className={styles.list} role="listbox" aria-label="Results">
          {results.map((command, index) => (
            <PaletteItem
              key={command.id}
              command={command}
              search={search}
              showGroup={command.group !== results[index - 1]?.group}
              isSelected={index === selected}
              onHover={() => {
                setSelected(index);
              }}
              onRun={() => {
                run(command);
              }}
            />
          ))}
        </ul>
      )}

      <div className={styles.footer}>
        <span>↑↓ move</span>
        <span>↵ go</span>
        <span>{shortcutLabel('n')} deploy with your agent</span>
        <span>{shortcutLabel('/')} all shortcuts</span>
      </div>
    </dialog>
  );
}

function PaletteItem({
  command,
  search,
  showGroup,
  isSelected,
  onHover,
  onRun,
}: {
  command: Command;
  search: string;
  showGroup: boolean;
  isSelected: boolean;
  onHover: () => void;
  onRun: () => void;
}) {
  const [before, match, after] = splitMatch(command.label, search);

  return (
    <>
      {showGroup && (
        <li className={styles.group} role="presentation">
          {command.group}
        </li>
      )}
      <li
        role="option"
        aria-selected={isSelected}
        className={cx(styles.item, isSelected && styles.selected)}
        onMouseEnter={onHover}
        onClick={onRun}
      >
        <span>
          {before}
          <strong>{match}</strong>
          {after}
        </span>
        <span className={styles.trail}>
          {command.shortcut && (
            <kbd className={styles.shortcut}>{shortcutLabel(command.shortcut)}</kbd>
          )}
          {isSelected && <span aria-hidden="true">↵</span>}
        </span>
      </li>
    </>
  );
}
