/**
 * The Projects screen before there are any projects, which is also where first run
 * happens: pick the providers your projects live on and connect each one, with the
 * Get started checklist beside it. Projects already running on a connected provider
 * are listed above, ready to add.
 */
import type { ReactNode } from 'react';
import { Link } from 'react-router-dom';
import { Button } from '@ui/components/Button';
import { ProviderTiles } from '@ui/components/ProviderTiles';
import { Mascot } from '@ui/components/Mascot';
import { PROVIDERS } from '@ui/lib/providers';
import type { ProjectsDialog } from './ProjectsDialogs';
import { GetStarted } from './GetStarted';
import styles from './EmptyProjects.module.css';

/**
 * `setOpen` opens a dialog owned by the Projects screen, so it stays open once
 * projects arrive. `found` lists what connected providers already have, ready to add.
 */
export function EmptyProjects({
  setOpen,
  found,
}: {
  setOpen: (dialog: ProjectsDialog) => void;
  found?: ReactNode;
}) {
  const tiles = (
    <ProviderTiles
      providers={PROVIDERS}
      onPick={(provider) => {
        setOpen({ kind: 'connect', provider });
      }}
    />
  );

  return (
    <div className={styles.screen}>
      <div className={styles.main}>
        {found}
        <Mascot mood="hello" size={64} />
        <h1 className={styles.title}>Where do your projects live?</h1>
        <p className={styles.lede}>
          Connect a provider and Hoista finds every project on it. Where the provider offers it,
          Connect takes you to its page to approve Hoista; otherwise you paste a token. Either way
          the access stays in this computer’s keychain, and Hoista changes nothing on the provider
          until you ask it to.
        </p>

        {tiles}

        <div className={styles.more}>
          <Button
            large
            variant="ghost"
            onClick={() => {
              setOpen({ kind: 'import' });
            }}
          >
            Bring Hoista over from another computer
          </Button>
          <Link to="/guides" className={styles.guides}>
            New to a provider? Step-by-step guides
          </Link>
          <span>More providers are on the way, one at a time.</span>
        </div>
      </div>
      <GetStarted />
    </div>
  );
}
