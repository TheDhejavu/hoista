/**
 * Settings, as a large window over the current screen: the sections on the
 * left, grouped into Hoista and This computer, and the chosen section on
 * the right under its title. Every change saves straight away.
 */
import {
  Link,
  Lock,
  MessageSquare,
  RotateCw,
  Server,
  Shield,
  SlidersHorizontal,
  Terminal,
  X,
  type LucideIcon,
} from 'lucide-react';
import { useState, type ReactNode } from 'react';
import { useSettings } from '@ui/api/settings';
import { Dialog } from '@ui/components/Dialog';
import type { SettingsSection } from '@ui/layout/useOverlays';
import { HelpDialog } from './HelpDialog';
import { cx } from '@ui/lib/cx';
import { AgentSettings, ManageAgentsButton } from './AgentSettings';
import { SafetySettings } from './SafetySettings';
import { CliSettings } from './CliSettings';
import { GeneralSettings } from './GeneralSettings';
import { InstanceSettings } from './InstanceSettings';
import { UpdateSettings } from './UpdateSettings';
import { VaultSettings } from './VaultSettings';
import { AddWebhookButton, WebhookSettings } from './WebhookSettings';
import styles from './SettingsDialog.module.css';

const GROUPS: {
  title: string;
  sections: { id: SettingsSection; label: string; icon: LucideIcon }[];
}[] = [
  {
    title: 'Hoista',
    sections: [
      { id: 'general', label: 'General', icon: SlidersHorizontal },
      { id: 'agents', label: 'Agents', icon: MessageSquare },
      { id: 'vault', label: 'Vault', icon: Lock },
      { id: 'safety', label: 'Safety', icon: Shield },
      { id: 'webhooks', label: 'Webhooks', icon: Link },
    ],
  },
  {
    title: 'This computer',
    sections: [
      { id: 'instances', label: 'Instances', icon: Server },
      { id: 'cli', label: 'Command line', icon: Terminal },
      { id: 'updates', label: 'Updates', icon: RotateCw },
    ],
  },
];

const TITLES = Object.fromEntries(
  GROUPS.flatMap((group) => group.sections).map((item) => [item.id, item.label]),
) as Record<SettingsSection, string>;

export function SettingsDialog({
  initialSection,
  onClose,
}: {
  initialSection: SettingsSection;
  onClose: () => void;
}) {
  const [section, setSection] = useState(initialSection);
  // Get help opens over Settings; closing it comes back here.
  const [helping, setHelping] = useState(false);
  const version = useSettings().data?.updates.current_version;

  return (
    <Dialog title="Settings" onClose={onClose} bare>
      <div className={styles.window}>
        <nav aria-label="Settings sections" className={styles.nav}>
          <div className={styles.navTitle} id="dialog-title">
            Settings
          </div>
          {GROUPS.map((group) => (
            <div key={group.title} className={styles.group}>
              <div className={styles.groupTitle}>{group.title}</div>
              {group.sections.map((item) => (
                <button
                  key={item.id}
                  type="button"
                  aria-current={item.id === section ? 'page' : undefined}
                  className={cx(styles.link, item.id === section && styles.active)}
                  onClick={() => {
                    setSection(item.id);
                  }}
                >
                  <item.icon size={16} strokeWidth={1.8} aria-hidden="true" />
                  {item.label}
                </button>
              ))}
            </div>
          ))}
          <div className={styles.navFoot}>
            Hoista {version ?? ''} ·{' '}
            <button
              type="button"
              className={styles.help}
              onClick={() => {
                setHelping(true);
              }}
            >
              Get help
            </button>
          </div>
        </nav>
        <div className={styles.main}>
          <header className={styles.head}>
            <h1>{TITLES[section]}</h1>
            {/* Vault has nothing to change here, only to read and act on. */}
            {section !== 'vault' && section !== 'webhooks' && (
              <span className={styles.saves}>Changes save as you go</span>
            )}
            <span className={styles.actions}>
              {sectionAction(section)}
              <button
                type="button"
                className={styles.close}
                aria-label="Close settings"
                onClick={onClose}
              >
                <X size={15} aria-hidden="true" />
              </button>
            </span>
          </header>
          <div className={styles.content}>{sectionContent(section)}</div>
        </div>
      </div>
      {helping && (
        <HelpDialog
          onClose={() => {
            setHelping(false);
          }}
        />
      )}
    </Dialog>
  );
}

/** What sits beside a section's title, when it has something to open. */
function sectionAction(section: SettingsSection): ReactNode {
  if (section === 'agents') return <ManageAgentsButton />;
  if (section === 'webhooks') return <AddWebhookButton />;
  return null;
}

function sectionContent(section: SettingsSection): ReactNode {
  switch (section) {
    case 'general':
      return <GeneralSettings />;
    case 'agents':
      return <AgentSettings />;
    case 'vault':
      return <VaultSettings />;
    case 'safety':
      return <SafetySettings />;
    case 'webhooks':
      return <WebhookSettings />;
    case 'instances':
      return <InstanceSettings />;
    case 'cli':
      return <CliSettings />;
    case 'updates':
      return <UpdateSettings />;
  }
}
