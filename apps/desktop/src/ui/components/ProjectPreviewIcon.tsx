/**
 * A tiny picture of a project's own page, used wherever a project needs an icon:
 * the sidebar, tables and headers. It reads as the app, not as a person, so
 * projects never get letter avatars. APIs and scheduled jobs look like a
 * terminal; everything else is a page in its own tint.
 */
import type { ProjectSummary } from '@shared/api';
import type { CSSProperties } from 'react';
import { projectLook, darkTint, isTerminal } from '@ui/lib/projectLook';
import styles from './ProjectPreviewIcon.module.css';

export function ProjectPreviewIcon({
  project,
  size = 20,
}: {
  project: Pick<ProjectSummary, 'id' | 'url'>;
  /** Height in pixels; the icon is a little wider than tall, like a screen. */
  size?: number;
}) {
  const look = projectLook(project);
  const terminal = isTerminal(look);
  const pad = Math.max(3, Math.round(size * 0.14));
  const lineHeight = Math.max(2, Math.round(size * 0.07));

  return (
    <span
      aria-hidden="true"
      className={styles.icon}
      style={{
        width: Math.round(size * 1.4),
        height: size,
        borderRadius: Math.round(size * 0.24),
        padding: `${pad}px ${pad}px 0`,
        ...tintVars(look.tint),
      }}
    >
      <span
        className={terminal ? styles.terminal : styles.page}
        style={{
          gap: lineHeight,
          padding: pad,
          borderRadius: `${Math.max(2, Math.round(size * 0.1))}px ${Math.max(2, Math.round(size * 0.1))}px 0 0`,
        }}
      >
        <span
          className={styles.bar}
          style={{
            height: Math.max(2, Math.round(size * 0.09)),
            background: terminal ? '#A3E635' : look.ink,
          }}
        />
        <span className={styles.line} style={{ height: lineHeight }} />
        <span className={styles.line} style={{ height: lineHeight, width: '60%' }} />
      </span>
    </span>
  );
}

/** The tint as CSS variables, so the stylesheet can pick the dark one. */
export function tintVars(tint: string): CSSProperties {
  return { '--tint': tint, '--tint-dark': darkTint(tint) } as CSSProperties;
}
