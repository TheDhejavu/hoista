/**
 * Providers as large tiles, each with what usually runs there and a Connect
 * label. Used on the empty Projects screen and under "Connect another provider" on
 * Accounts.
 */
import type { ProviderId } from '@shared/api';
import { serviceName } from '@shared/providers';
import type { Provider } from '@ui/lib/providers';
import { ServiceIcon } from './ServiceIcon';
import styles from './ProviderTiles.module.css';

export function ProviderTiles({
  providers,
  onPick,
  label = 'Providers',
}: {
  providers: Provider[];
  onPick: (provider: ProviderId) => void;
  label?: string;
}) {
  return (
    <ul className={styles.grid} aria-label={label}>
      {providers.map((provider) => (
        <ProviderTile key={provider.id} provider={provider} onPick={onPick} />
      ))}
    </ul>
  );
}

function ProviderTile({
  provider,
  onPick,
}: {
  provider: Provider;
  onPick: (provider: ProviderId) => void;
}) {
  return (
    <li>
      <button
        type="button"
        className={styles.tile}
        onClick={() => {
          onPick(provider.id);
        }}
      >
        <span className={styles.logo}>
          <ServiceIcon id={provider.id} size={18} />
        </span>
        <span className={styles.text}>
          <span>{serviceName(provider.id)}</span>
          <span className={styles.for}>{provider.for}</span>
        </span>
        <span className={styles.connect}>Connect</span>
      </button>
    </li>
  );
}
