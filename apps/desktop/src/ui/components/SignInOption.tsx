/**
 * Connecting or reconnecting a provider on its own page: Hoista opens it, you
 * click Allow there, and the access comes back to Hoista. Shown only for providers that offer it; the token way sits below.
 */
import type { ServiceId, SignInOption as SignInDetails, TokenCheck } from '@shared/api';
import { serviceName } from '@shared/providers';
import { useSignInAccount } from '@ui/api/accounts';
import { Button } from './Button';
import { ServiceIcon } from './ServiceIcon';
import { ErrorMessage } from './ErrorMessage';
import { cx } from '@ui/lib/cx';
import styles from './SignInOption.module.css';

export function SignInOption({
  service,
  details,
  again = false,
  wide = false,
  onConnected,
}: {
  service: ServiceId;
  details: SignInDetails;
  /** Reconnecting an account that stopped working, rather than adding one. */
  again?: boolean;
  /** Puts the button beside the text, for full-width pages. */
  wide?: boolean;
  onConnected: (checks: TokenCheck[]) => void;
}) {
  const signIn = useSignInAccount();
  const name = serviceName(service);

  return (
    <section className={cx(styles.option, wide && styles.wide)}>
      <div>
        <div className={styles.head}>
          <b>{again ? `Reconnect on ${name}’s page` : `Connect on ${name}’s page`}</b>
        </div>
        <p>
          Opens {name} in your browser. Click Allow there and you’re done. {details.on_their_page}
        </p>
        <ErrorMessage error={signIn.error} />
      </div>
      <Button
        variant="accent"
        large
        disabled={signIn.isPending}
        onClick={() => {
          signIn.mutate(
            { service },
            {
              onSuccess: (result) => {
                onConnected(result.checks);
              },
            },
          );
        }}
      >
        <span className={styles.logo} aria-hidden="true">
          <ServiceIcon id={service} size={14} />
        </span>
        {signIn.isPending ? 'Waiting for you to click Allow' : `Continue with ${name}`}
      </Button>
    </section>
  );
}

/** The line between connecting on the provider's page and the token way. */
export function OrUseToken({ label = 'or use a token' }: { label?: string }) {
  return (
    <div className={styles.or}>
      <span>{label}</span>
    </div>
  );
}
