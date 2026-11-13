/**
 * Setup step 1, second half: one card per picked provider. Connected ones
 * say which account and how many projects Hoista found; the one being
 * connected offers its own page to sign in on (where the provider has one)
 * or a token, which opens in place. Others wait their turn.
 */
import { useState, type ReactNode } from 'react';
import { Navigate, useNavigate } from 'react-router-dom';
import { ArrowUpRight, Check, Lock } from 'lucide-react';
import type { ConnectGuide, ProviderId, TokenCheck } from '@shared/api';
import { entryOf, serviceName } from '@shared/providers';
import { useAccounts, useConnectGuide } from '@ui/api/accounts';
import { useFoundOnProviders } from '@ui/api/imports';
import { openExternal } from '@ui/api/system';
import { Button } from '@ui/components/Button';
import { ErrorMessage } from '@ui/components/ErrorMessage';
import { QueryState } from '@ui/components/QueryState';
import { TokenChecks } from '@ui/components/TokenChecks';
import { cx } from '@ui/lib/cx';
import { plural } from '@ui/lib/format';
import { providerGuide } from '@ui/lib/providerGuides';
import { ServiceLogo } from '@ui/screens/accounts/ServiceLogo';
import {
  TokenAccountFields,
  useBrowserSignIn,
  useTokenConnect,
} from '@ui/screens/accounts/useConnect';
import { ProviderGuideDialog } from '@ui/screens/guides/ProviderGuideScreen';
import { projectsFromFound } from '@ui/screens/projects/usePicker';
import { SetupStep } from './SetupStep';
import { stepPath, usePicked } from './useSetup';
import styles from './Setup.module.css';

/** 1Password's own syntax for a secret reference. */
const REFERENCE_EXAMPLE = 'op://Vault/Item/field';

export function ConnectProvidersStep() {
  const navigate = useNavigate();
  const picked = usePicked();
  const accounts = useAccounts();
  // The card being connected: the one you clicked or the one after the last
  // connected, else (undefined) the first not yet connected. It stays open
  // until it has said how connecting went, even once the account is there.
  const [chosen, setChosen] = useState<ProviderId | null | undefined>(undefined);
  // Connected, but some of the engine's checks need a look.
  const [gaps, setGaps] = useState<Partial<Record<ProviderId, TokenCheck[]>>>({});

  if (picked.length === 0) return <Navigate to={stepPath('providers', picked)} replace />;

  const items = accounts.data?.items;
  const connected = new Set<string>((items ?? []).map((item) => item.service));
  const done = picked.filter((id) => connected.has(id)).length;
  const current = chosen === undefined ? picked.find((id) => !connected.has(id)) : chosen;
  // Held from the start, so the card doesn't move on before it has reported.
  if (chosen === undefined && items && current) setChosen(current);

  function finished(id: ProviderId, checks: TokenCheck[]): void {
    if (!checks.every((check) => check.ok)) setGaps((all) => ({ ...all, [id]: checks }));
    setChosen(picked.find((other) => other !== id && !connected.has(other)) ?? null);
  }

  return (
    <SetupStep
      number={1}
      title="Connect your providers"
      lede="Each access stays in this computer’s keychain. Agents never see it."
      back={
        <Button
          size="xl"
          onClick={() => {
            void navigate(stepPath('providers', picked));
          }}
        >
          Back
        </Button>
      }
      note={items && `${String(done)} of ${String(picked.length)} connected`}
      actions={
        <Button
          variant="accent"
          size="xl"
          disabled={done === 0}
          onClick={() => {
            void navigate(stepPath('projects', picked));
          }}
        >
          Continue
        </Button>
      }
    >
      <QueryState query={accounts}>
        {() => (
          <ul className={styles.connectList} aria-label="Providers to connect">
            {/* Connected ones first, the way the list fills in from the top. */}
            {[...picked]
              .sort((a, b) => Number(connected.has(b)) - Number(connected.has(a)))
              .map((id) => (
                <li key={id}>
                  {id === current ? (
                    <ConnectingCard
                      service={id}
                      onConnected={(checks) => {
                        finished(id, checks);
                      }}
                    />
                  ) : connected.has(id) ? (
                    <ConnectedCard service={id} checks={gaps[id] ?? null} />
                  ) : (
                    <WaitingCard
                      service={id}
                      onConnect={() => {
                        setChosen(id);
                      }}
                    />
                  )}
                </li>
              ))}
          </ul>
        )}
      </QueryState>
    </SetupStep>
  );
}

/** Logo, name and a line under it: the top of every card. */
function CardHead({
  service,
  text,
  children,
}: {
  service: ProviderId;
  text: string;
  children?: ReactNode;
}) {
  return (
    <div className={styles.cardHead}>
      <ServiceLogo id={service} name={serviceName(service)} size={40} />
      <span className={styles.cardText}>
        <span className={styles.cardName}>{serviceName(service)}</span>
        <span className={styles.cardSub}>{text}</span>
      </span>
      {children}
    </div>
  );
}

/** Connected: which account, and what Hoista found on it. */
function ConnectedCard({ service, checks }: { service: ProviderId; checks: TokenCheck[] | null }) {
  const accounts = useAccounts();
  const found = useFoundOnProviders();
  const account = (accounts.data?.items ?? [])
    .filter((item) => item.service === service)
    .sort((a, b) => b.connected_at.localeCompare(a.connected_at))[0];
  // A code account (GitHub) runs nothing, so there's nothing to count.
  const code = entryOf(service).catalogue.category === 'code';
  const count = found.data ? projectsFromFound(found.data, service).length : null;
  const what = code
    ? 'repositories your projects build from'
    : count === null
      ? 'looking for projects'
      : `found ${plural(count, 'project')}`;

  return (
    <div className={styles.card}>
      <CardHead service={service} text={`Connected ${account?.label ?? ''} · ${what}`}>
        <span className={styles.connected}>
          <span className={styles.connectedBadge}>
            <Check size={12} strokeWidth={3.5} aria-hidden="true" />
          </span>
          Connected
        </span>
      </CardHead>
      {checks && checks.some((check) => !check.ok) && (
        <div className={styles.gaps}>
          <span>Some checks need a look before everything works.</span>
          <TokenChecks checks={checks} />
        </div>
      )}
    </div>
  );
}

/** Not connected yet, and not the one being connected now. */
function WaitingCard({ service, onConnect }: { service: ProviderId; onConnect: () => void }) {
  return (
    <div className={styles.card}>
      <CardHead service={service} text={entryOf(service).catalogue.description}>
        <Button large onClick={onConnect}>
          Connect
        </Button>
      </CardHead>
    </div>
  );
}

/** The one being connected: its guide first, then a way in. */
function ConnectingCard({
  service,
  onConnected,
}: {
  service: ProviderId;
  onConnected: (checks: TokenCheck[]) => void;
}) {
  const guide = useConnectGuide(service);
  return (
    <div className={cx(styles.card, styles.cardNow)}>
      <QueryState query={guide}>
        {(data) => <ConnectWays guide={data} onConnected={onConnected} />}
      </QueryState>
    </div>
  );
}

function ConnectWays({
  guide,
  onConnected,
}: {
  guide: ConnectGuide;
  /** Connected, with the engine's checks; the card then closes. */
  onConnected: (checks: TokenCheck[]) => void;
}) {
  const [withToken, setWithToken] = useState(guide.sign_in === null);
  // The provider's step-by-step guide, open over setup.
  const [guideStep, setGuideStep] = useState<'account' | 'token' | null>(null);
  const hasGuide = Boolean(providerGuide(guide.service));
  const signIn = useBrowserSignIn(guide, onConnected);
  const token = useTokenConnect(guide, onConnected);
  const reference = token.token.trim().startsWith('op://');

  const noAccount = hasGuide ? (
    <button
      type="button"
      className={styles.textLink}
      onClick={() => {
        setGuideStep('account');
      }}
    >
      No account yet? Walk me through it
    </button>
  ) : (
    <button
      type="button"
      className={styles.textLink}
      onClick={() => {
        void openExternal(guide.signup_url);
      }}
    >
      No account yet? Make one ↗
    </button>
  );

  return (
    <>
      <CardHead service={guide.service} text={guide.intro}>
        <span className={styles.next}>Next</span>
      </CardHead>
      {withToken ? (
        <div className={styles.tokenWay}>
          <div className={styles.row}>
            <Button
              large
              onClick={() => {
                void openExternal(guide.token_url);
              }}
            >
              Open the tokens page
              <ArrowUpRight size={13} strokeWidth={2.2} aria-hidden="true" />
            </Button>
            {hasGuide ? (
              <button
                type="button"
                className={styles.textLink}
                onClick={() => {
                  setGuideStep('token');
                }}
              >
                Step-by-step guide
              </button>
            ) : (
              noAccount
            )}
            {/* Without a sign-in way, this is the only place to ask for help making an account. */}
            {hasGuide && !guide.sign_in && noAccount}
          </div>
          <span className={styles.tokenSteps}>{guide.token_steps}</span>
          <form
            className={styles.tokenRow}
            onSubmit={(event) => {
              event.preventDefault();
              if (token.canSubmit) token.submit();
            }}
          >
            <label className={styles.tokenField}>
              <span className="visually-hidden">{guide.token_label}</span>
              <input
                type="password"
                autoComplete="off"
                spellCheck={false}
                value={token.token}
                placeholder={`${guide.token_placeholder}, or ${REFERENCE_EXAMPLE}`}
                onChange={(event) => {
                  token.setToken(event.target.value);
                }}
              />
            </label>
            <Button
              type="submit"
              variant="accent"
              size="xl"
              busy={token.isBusy}
              busyLabel="Checking"
              disabled={!token.canSubmit}
            >
              {token.actionLabel}
            </Button>
          </form>
          <TokenAccountFields connect={token} />
          <span className={styles.fine}>
            {reference
              ? 'A 1Password reference: the token stays in 1Password, and Hoista checks it works before saving.'
              : 'Saved to this computer’s keychain. Hoista checks it works before saving.'}{' '}
            {guide.sign_in && (
              <button
                type="button"
                className={styles.textLink}
                onClick={() => {
                  setWithToken(false);
                }}
              >
                Sign in on {guide.name}’s page instead
              </button>
            )}
          </span>
        </div>
      ) : (
        <>
          <div className={styles.row}>
            <Button variant="accent" size="xl" onClick={signIn.open}>
              {signIn.isPending
                ? `Open ${guide.name}’s page again`
                : `Sign in on ${guide.name}’s page`}
              <ArrowUpRight size={13} strokeWidth={2.2} aria-hidden="true" />
            </Button>
            <Button
              size="xl"
              onClick={() => {
                setWithToken(true);
              }}
            >
              Paste a token
            </Button>
            {noAccount}
          </div>
          {signIn.isPending && (
            <p className={styles.waiting} role="status">
              <span className={styles.spinner} aria-hidden="true" />
              Waiting for you to click Allow on {guide.name}’s page
            </p>
          )}
          <ErrorMessage error={signIn.error} />
          <span className={styles.fine}>
            <Lock size={13} aria-hidden="true" />
            Or keep it in 1Password and paste its secret reference instead.
          </span>
        </>
      )}
      {guideStep && (
        <ProviderGuideDialog
          provider={guide.service}
          step={guideStep}
          onClose={() => {
            setGuideStep(null);
          }}
        />
      )}
    </>
  );
}
