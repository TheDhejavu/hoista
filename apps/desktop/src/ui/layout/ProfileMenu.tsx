/**
 * The profile button at the bottom of the sidebar, and the small menu that
 * opens above it. The button also says which instance the app shows, with a
 * dot on the avatar for whether agents can connect. Settings, Get help and What's new open as dialogs over the
 * current screen.
 *
 * The menu also switches instance: this Mac's Hoista, or one of your
 * remote instances on servers of yours, and adds one.
 * There is no sign-in to Hoista itself: the profile only names "You".
 */
import { Check, ChevronsUpDown, Laptop, Plus, Server } from 'lucide-react';
import { useEffect, useRef, useState, type RefObject } from 'react';
import { useProfile } from '@ui/api/accounts';
import { useRemotes, useSettings, useShowRemote, useShownRemote } from '@ui/api/settings';
import { quitProject } from '@ui/api/system';
import { ExternalLink } from '@ui/components/ExternalLink';
import { Dot } from '@ui/components/Dot';
import { cx } from '@ui/lib/cx';
import { AddRemoteDialog } from '@ui/screens/settings/remote/AddRemoteDialog';
import { remoteState } from '@ui/screens/settings/remote/RemotesList';
import { useOverlays } from './useOverlays';
import styles from './ProfileMenu.module.css';

const SOURCE_URL = 'https://github.com/TheDhejavu/hoista';

export function ProfileMenuButton() {
  const container = useRef<HTMLDivElement>(null);
  const [isOpen, setOpen] = useState(false);
  // Opened from the menu, kept here so it stays open once the menu closes.
  const [adding, setAdding] = useState<{ signInTo?: string } | null>(null);
  const profile = useProfile();
  const status = useAgentStatus();

  useCloseOnOutsideClick(container, () => {
    setOpen(false);
  });

  return (
    <div ref={container} className={styles.container}>
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={isOpen}
        className={styles.button}
        title={status.summary}
        onClick={() => {
          setOpen((open) => !open);
        }}
      >
        <span className={styles.buttonAvatar}>
          {profile.data?.initials ?? ''}
          <span className={styles.avatarDot}>
            <Dot tone={status.canConnect ? 'good' : 'off'} small />
          </span>
        </span>
        <span className={styles.buttonText}>
          <span className={styles.name}>{profile.data?.name ?? ''}</span>
          <span className={styles.mode}>{status.mode}</span>
        </span>
        <ChevronsUpDown size={14} className={styles.chevron} aria-hidden="true" />
      </button>
      {isOpen && (
        <ProfileMenu
          onClose={() => {
            setOpen(false);
          }}
          onAdd={(signInTo) => {
            setOpen(false);
            setAdding(signInTo ? { signInTo } : {});
          }}
        />
      )}
      {adding && (
        <AddRemoteDialog
          {...(adding.signInTo ? { signInTo: adding.signInTo } : {})}
          onClose={() => {
            setAdding(null);
          }}
        />
      )}
    </div>
  );
}

function ProfileMenu({
  onClose,
  onAdd,
}: {
  onClose: () => void;
  /** Adds a remote, or signs this Mac in to the one given. */
  onAdd: (signInTo?: string) => void;
}) {
  const profile = useProfile();
  const newVersion = useNewVersion();
  const { openSettings, openHelp } = useOverlays();
  const status = useAgentStatus();

  /** Runs a menu choice, then closes the menu. */
  function choose(action: () => unknown): () => void {
    return () => {
      onClose();
      void action();
    };
  }

  return (
    <div role="menu" aria-label="Profile" className={styles.menu}>
      <div className={styles.who}>
        <span className={styles.avatar} aria-hidden="true">
          {profile.data?.initials}
        </span>
        <span className={styles.whoText}>
          <span>{profile.data?.name}</span>
          <span className={styles.muted}>Profile on this computer</span>
        </span>
      </div>
      <hr className={styles.divider} />
      <HoistaSwitcher onClose={onClose} onAdd={onAdd} />
      <hr className={styles.divider} />
      {/* Only worth a line when it's off: the dot on the avatar says the rest. */}
      {!status.canConnect && !status.unreachable && (
        <>
          <MenuItem
            label="Agent connection is off"
            hint="Turn on"
            highlight
            onClick={choose(() => {
              openSettings('agents');
            })}
          />
          <hr className={styles.divider} />
        </>
      )}
      <MenuItem
        label="Settings"
        hint="⌘,"
        onClick={choose(() => {
          openSettings();
        })}
      />
      <MenuItem label="Get help" onClick={choose(openHelp)} />
      <MenuItem
        label="What's new"
        hint={newVersion && `v${newVersion} ready`}
        highlight
        onClick={choose(() => {
          openSettings('updates');
        })}
      />
      <ExternalLink url={SOURCE_URL} variant="text" className={styles.item}>
        Hoista on GitHub
      </ExternalLink>
      <hr className={styles.divider} />
      <MenuItem label="Quit Hoista" hint="⌘Q" muted onClick={choose(quitProject)} />
    </div>
  );
}

/**
 * Which Hoista the app shows: this Mac's, or one of your remotes. Picking
 * one switches every screen; one this Mac isn't signed in to yet opens its
 * sign-in instead.
 */
function HoistaSwitcher({
  onClose,
  onAdd,
}: {
  onClose: () => void;
  onAdd: (signInTo?: string) => void;
}) {
  const remotes = useRemotes();
  const show = useShowRemote();
  const showing = remotes.data?.showing ?? 'this_computer';

  function pick(id: string): void {
    if (id === showing) {
      onClose();
      return;
    }
    show.mutate(id, { onSuccess: onClose });
  }

  return (
    <div role="group" aria-label="Instances" className={styles.switcher}>
      <span className={styles.groupLabel}>Instances</span>
      <button
        type="button"
        role="menuitemradio"
        aria-checked={showing === 'this_computer'}
        className={styles.place}
        onClick={() => {
          pick('this_computer');
        }}
      >
        <span className={styles.placeIcon}>
          <Laptop size={14} aria-hidden="true" />
        </span>
        <span className={styles.placeText}>
          <span className={styles.placeName}>This Mac</span>
        </span>
        <PlaceMark
          busy={show.isPending && show.variables === 'this_computer'}
          on={showing === 'this_computer'}
        />
      </button>
      {remotes.isLoading && <span className={styles.placeLoading}>Loading your remotes</span>}
      {remotes.data?.items.map((remote) => {
        const state = remoteState(remote);
        return (
          <button
            key={remote.id}
            type="button"
            role="menuitemradio"
            aria-checked={showing === remote.id}
            className={styles.place}
            title={`${remote.url} · ${state.label}`}
            onClick={() => {
              if (remote.signed_in) pick(remote.id);
              else onAdd(remote.id);
            }}
          >
            <span className={styles.placeIcon}>
              <Server size={14} aria-hidden="true" />
              <span className={styles.placeDot}>
                <Dot tone={state.tone} small />
              </span>
            </span>
            <span className={styles.placeText}>
              <span className={styles.placeName}>{remote.name}</span>
              <span className={styles.placeSub}>
                {remote.signed_in ? remote.url.replace(/^https?:\/\//, '') : 'Signed out'}
              </span>
            </span>
            <PlaceMark
              busy={show.isPending && show.variables === remote.id}
              on={showing === remote.id}
            />
          </button>
        );
      })}
      <button
        type="button"
        role="menuitem"
        className={cx(styles.place, styles.addPlace)}
        onClick={() => {
          onAdd();
        }}
      >
        <span className={styles.placeIcon}>
          <Plus size={14} aria-hidden="true" />
        </span>
        <span className={styles.placeText}>
          <span className={styles.placeName}>Add a remote</span>
        </span>
      </button>
    </div>
  );
}

/** A tick on the one shown, a spinner on the one switching to. */
function PlaceMark({ on, busy }: { on: boolean; busy: boolean }) {
  if (busy) return <span className={styles.placeSpinner} aria-label="Switching" />;
  return on ? <Check size={14} className={styles.placeCheck} aria-label="Current" /> : <span />;
}

function MenuItem({
  label,
  hint,
  highlight = false,
  muted = false,
  onClick,
}: {
  label: string;
  hint?: string | null;
  /** Shows the hint in the accent colour, for news worth noticing. */
  highlight?: boolean;
  muted?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      className={cx(styles.item, muted && styles.mutedItem)}
      onClick={onClick}
    >
      {label}
      {hint && <span className={highlight ? styles.news : styles.muted}>{hint}</span>}
    </button>
  );
}

/** Which instance the app shows (under your name), and whether agents can connect. */
function useAgentStatus(): {
  canConnect: boolean;
  unreachable: boolean;
  mode: string;
  summary: string;
} {
  const agents = useSettings().data?.agents;
  const current = useShownRemote();
  const shown = current?.remote;
  // A remote that isn't answering: agents can't reach it either.
  const unreachable = current?.down === true;
  const canConnect = (agents?.connection_enabled ?? true) && !unreachable;
  const mode = shown ? shown.name : 'This Mac';
  return {
    canConnect,
    unreachable,
    mode,
    summary: `${mode} · ${
      unreachable ? 'Not answering' : canConnect ? 'Agents can connect' : 'Agent connection is off'
    }`,
  };
}

/** The newer version waiting to install, if there is one. */
function useNewVersion(): string | null {
  const updates = useSettings().data?.updates;
  if (!updates?.latest_version || updates.latest_version === updates.current_version) return null;
  return updates.latest_version;
}

/** Calls `onClose` on a click outside `container`, or on Escape. */
function useCloseOnOutsideClick(
  container: RefObject<HTMLDivElement | null>,
  onClose: () => void,
): void {
  useEffect(() => {
    function onPointerDown(event: PointerEvent): void {
      if (event.target instanceof Node && !container.current?.contains(event.target)) onClose();
    }
    function onKeyDown(event: KeyboardEvent): void {
      if (event.key === 'Escape') onClose();
    }
    document.addEventListener('pointerdown', onPointerDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('pointerdown', onPointerDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [container, onClose]);
}
