/**
 * Opens a page in the user's browser, never inside the app. Adds the ↗
 * that marks every link leaving Hoista.
 */
import type { ReactNode } from 'react';
import { openExternal } from '@ui/api/system';
import { Button } from './Button';
import styles from './ExternalLink.module.css';

export function ExternalLink({
  url,
  children,
  variant = 'button',
  className,
}: {
  url: string;
  children: ReactNode;
  /** `button` looks like a secondary button; `text` like an underlined link. */
  variant?: 'button' | 'text';
  /** Replaces the `text` variant's look, for links styled by their container. */
  className?: string;
}) {
  function open(): void {
    void openExternal(url);
  }

  if (variant === 'text') {
    return (
      <button type="button" className={className ?? styles.text} onClick={open}>
        {children} ↗
      </button>
    );
  }
  return <Button onClick={open}>{children} ↗</Button>;
}
