/**
 * Setup step 1: every provider Hoista connects to, as cards with a checkbox. The
 * ones already connected start ticked. Next, each picked one gets connected.
 */
import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { Check } from 'lucide-react';
import type { ProviderId } from '@shared/api';
import { serviceName } from '@shared/providers';
import { useAccounts } from '@ui/api/accounts';
import { Button } from '@ui/components/Button';
import { cx } from '@ui/lib/cx';
import { listOf, plural } from '@ui/lib/format';
import { PROVIDERS } from '@ui/lib/providers';
import { ServiceLogo } from '@ui/screens/accounts/ServiceLogo';
import { SetupStep } from './SetupStep';
import { stepPath, usePicked } from './useSetup';
import styles from './Setup.module.css';

export function PickProvidersStep() {
  const navigate = useNavigate();
  const fromAddress = usePicked();
  const accounts = useAccounts();
  // Until you change the picks: what the address says, else what's already connected.
  const [chosen, setChosen] = useState<ProviderId[] | null>(null);
  const connected = new Set<string>((accounts.data?.items ?? []).map((item) => item.service));
  const picked =
    chosen ??
    (fromAddress.length > 0
      ? fromAddress
      : PROVIDERS.map((provider) => provider.id).filter((id) => connected.has(id)));

  function toggle(id: ProviderId): void {
    const next = picked.includes(id) ? picked.filter((item) => item !== id) : [...picked, id];
    // Kept in the list's order, so the next step shows them the same way.
    setChosen(PROVIDERS.map((provider) => provider.id).filter((item) => next.includes(item)));
  }

  return (
    <SetupStep
      number={1}
      title="Where do your projects live?"
      lede="Pick every provider you use. Next you connect each one, and Hoista finds your projects on it. Nothing changes on them until you ask."
      back={
        <Button
          size="xl"
          onClick={() => {
            void navigate('/welcome');
          }}
        >
          Back
        </Button>
      }
      note={
        picked.length > 0
          ? `${listOf(picked.map((id) => serviceName(id)))} picked`
          : 'Pick at least one'
      }
      actions={
        <Button
          variant="accent"
          size="xl"
          disabled={picked.length === 0}
          onClick={() => {
            void navigate(stepPath('connect', picked));
          }}
        >
          {picked.length > 0 ? `Connect ${plural(picked.length, 'provider')}` : 'Connect providers'}
        </Button>
      }
    >
      <ul className={styles.providerGrid} aria-label="Providers">
        {PROVIDERS.map((provider) => {
          const on = picked.includes(provider.id);
          return (
            <li key={provider.id}>
              <button
                type="button"
                className={cx(styles.providerCard, on && styles.providerOn)}
                aria-pressed={on}
                onClick={() => {
                  toggle(provider.id);
                }}
              >
                <span className={styles.tick} aria-hidden="true">
                  <Check size={11} strokeWidth={3.5} />
                </span>
                <ServiceLogo id={provider.id} name={serviceName(provider.id)} size={34} />
                <span className={styles.providerText}>
                  <span className={styles.providerName}>{serviceName(provider.id)}</span>
                  <span className={styles.providerFor}>{provider.for}</span>
                </span>
              </button>
            </li>
          );
        })}
      </ul>
    </SetupStep>
  );
}
