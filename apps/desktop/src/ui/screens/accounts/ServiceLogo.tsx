/**
 * A rounded tile with a service's logo, or its first letter for services
 * without one. `size` is the tile's width; the logo is drawn at half of it.
 */
import { logoFor } from '@shared/providers';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import styles from './ServiceLogo.module.css';

export function ServiceLogo({ id, name, size }: { id: string; name: string; size?: number }) {
  const style = size
    ? { width: size, height: size, borderRadius: Math.round(size * 0.27) }
    : undefined;
  return (
    <span className={styles.logo} style={style} aria-hidden="true">
      {logoFor(id) ? (
        <ServiceIcon id={id} size={size ? Math.round(size / 2) : 18} />
      ) : (
        name.charAt(0)
      )}
    </span>
  );
}
