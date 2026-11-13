/**
 * A service's logo, such as Cloudflare or Neon, drawn in its brand colour.
 * Draws nothing for a service without a logo.
 */
import { logoFor } from '@shared/providers';
import styles from './ServiceIcon.module.css';

export function ServiceIcon({
  id,
  size = 18,
  mono = false,
}: {
  id: string;
  size?: number;
  /** Drawn in the text colour around it, for logos on a filled button. */
  mono?: boolean;
}) {
  const logo = logoFor(id);
  if (!logo) return null;

  return (
    <svg className={styles.icon} width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
      <path d={logo.path} fill={mono ? 'currentColor' : logo.colour} fillRule={logo.fillRule} />
    </svg>
  );
}
