/**
 * One provider guide, at `/guides/:provider/account` or `/guides/:provider/token`: the
 * three steps on the left, a picture of the provider's page with what to do
 * there, and the buttons to open the real page and to paste the token into
 * Hoista. Providers that offer it also get connecting on their own page above the
 * token steps.
 */
import { Check, X } from 'lucide-react';
import { useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { serviceName } from '@shared/providers';
import type { TokenPermission } from '@shared/api';
import { useConnectGuide } from '@ui/api/accounts';
import { openExternal } from '@ui/api/system';
import { Button } from '@ui/components/Button';
import { Dialog } from '@ui/components/Dialog';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { SignInOption } from '@ui/components/SignInOption';
import { cx } from '@ui/lib/cx';
import { providerGuide, type ProviderGuide as ProviderGuideData } from '@ui/lib/providerGuides';
import { NotFound } from '../NotFound';
import { BrowserFrame, SignupPage, TokenPage } from './ProviderPages';
import styles from './ProviderGuideScreen.module.css';

/**
 * Real screenshots of each provider's token page, taken at desktop size, with
 * personal details blurred. Providers without one fall back to the drawing.
 */
const TOKEN_SHOTS = import.meta.glob<string>('../../assets/guides/*-token.webp', {
  eager: true,
  query: '?url',
  import: 'default',
});

function tokenShot(provider: string): string | undefined {
  return TOKEN_SHOTS[`../../assets/guides/${provider}-token.webp`];
}

/** When the screenshots were taken, so it's clear how fresh they are. */
const SHOTS_TAKEN = 'November 2026';

export function ProviderGuideScreen() {
  const { provider, step } = useParams();
  const navigate = useNavigate();
  const guide = providerGuide(provider);
  if (!guide || (step !== 'account' && step !== 'token')) return <NotFound />;
  return (
    <ProviderGuide
      guide={guide}
      isAccount={step === 'account'}
      to={{
        step: (next) => void navigate(`/guides/${guide.provider}/${next}`),
        paste: () => void navigate(`/accounts/connect/${guide.provider}`),
        connected: () => void navigate('/accounts'),
        allGuides: () => void navigate('/guides'),
      }}
    />
  );
}

/**
 * The same guide over whatever the person was doing (connecting an account,
 * replacing a token), so they never lose their place. Closing it, or
 * "I've copied it", goes back to where they were.
 */
export function ProviderGuideDialog({
  provider,
  step,
  onClose,
  onConnected,
}: {
  provider: string;
  step: 'account' | 'token';
  onClose: () => void;
  /** Connected on the provider's own page from inside the guide. */
  onConnected?: () => void;
}) {
  const [at, setAt] = useState(step);
  const guide = providerGuide(provider);
  if (!guide) return null;
  return (
    <Dialog title={`${serviceName(guide.provider)} guide`} onClose={onClose} bare large>
      <div className={styles.inDialog}>
        <button
          type="button"
          className={styles.close}
          aria-label="Close the guide"
          onClick={onClose}
        >
          <X size={16} />
        </button>
        <ProviderGuide
          guide={guide}
          isAccount={at === 'account'}
          to={{
            step: setAt,
            paste: onClose,
            connected: () => {
              onClose();
              onConnected?.();
            },
            allGuides: null,
          }}
        />
      </div>
    </Dialog>
  );
}

/** Where the guide's buttons go: other pages on the Guides screen, or back into a dialog. */
interface GuideMoves {
  step: (step: 'account' | 'token') => void;
  /** "I've copied it": on to pasting the token. */
  paste: () => void;
  connected: () => void;
  /** Back to every guide; none inside a dialog. */
  allGuides: (() => void) | null;
}

function ProviderGuide({
  guide,
  isAccount,
  to,
}: {
  guide: ProviderGuideData;
  isAccount: boolean;
  to: GuideMoves;
}) {
  const connect = useConnectGuide(guide.provider).data;
  const signInOption = connect?.sign_in ?? null;
  // The provider's real page, as the engine declares it.
  const page = connect && (isAccount ? connect.signup_url : connect.token_url);

  const name = serviceName(guide.provider);
  const steps = [
    { title: `Make a ${name} account`, text: guide.plan },
    signInOption
      ? { title: 'Connect Hoista', text: 'On their page, or a token' }
      : { title: 'Create a token', text: 'Takes about a minute' },
    signInOption
      ? { title: 'Hoista checks access', text: 'Kept in your keychain' }
      : { title: 'Paste it into Hoista', text: 'Kept in your keychain' },
  ];
  const at = isAccount ? 0 : 1;
  const shot = tokenShot(guide.provider);
  const firstOption = guide.signupOptions[0] ?? '';
  const legend = isAccount
    ? [
        firstOption.includes('GitHub')
          ? 'Pick a way to sign up. GitHub is quickest if your code is on GitHub.'
          : `Pick a way to sign up. ${firstOption} is quickest.`,
        'Confirm the email they send you',
      ]
    : guide.tokenSteps;

  return (
    <div className={styles.page}>
      <aside className={styles.rail}>
        {to.allGuides && (
          <Link to="/guides" className={styles.back}>
            ‹ Provider guides
          </Link>
        )}
        <div className={styles.railHead}>
          <span className={styles.logo}>
            <ServiceIcon id={guide.provider} size={18} />
          </span>
          <b>Connect {name}</b>
        </div>
        <ol className={styles.steps}>
          {steps.map((item, index) => (
            <li key={item.title} className={cx(index > at && styles.later)}>
              <span className={cx(styles.badge, index <= at && styles.on)}>
                {index < at ? <Check size={12} strokeWidth={3.5} /> : index + 1}
              </span>
              <span>
                <b>{item.title}</b>
                <span className={styles.muted}>{item.text}</span>
              </span>
            </li>
          ))}
        </ol>
        <div className={styles.railNotes}>
          {guide.headsUp && <p className={styles.headsUp}>{guide.headsUp}</p>}
          <p className={styles.note}>
            Pages on {name} can change. If yours looks different, the names in the steps still
            match.
          </p>
        </div>
      </aside>

      <div className={styles.main}>
        <h1>
          {isAccount
            ? `Make a ${name} account`
            : signInOption
              ? `Connect Hoista to ${name}`
              : 'Create a token for Hoista'}
        </h1>
        <p className={styles.lede}>
          {isAccount
            ? `Hoista uses ${name} for ${guide.usedFor.charAt(0).toLowerCase()}${guide.usedFor.slice(1)}. Sign up on their site, confirm your email, then come back here.`
            : signInOption
              ? `Two ways to do it. Either way, Hoista checks what it can see first and asks you before changing anything.`
              : `A token lets Hoista reach your ${name} account. Hoista checks what it can see first and asks you before changing anything.`}
        </p>
        {!isAccount && signInOption && (
          <>
            <div className={styles.oneClick}>
              <SignInOption
                service={guide.provider}
                details={signInOption}
                wide
                onConnected={to.connected}
              />
            </div>
            <h2 className={styles.orHead}>Or create a token</h2>
          </>
        )}
        <div className={styles.shotWrap}>
          <BrowserFrame url={page?.replace(/^https:\/\//, '') ?? ''} fit={Boolean(shot)}>
            {isAccount ? (
              <SignupPage guide={guide} />
            ) : shot ? (
              <img className={styles.shot} src={shot} alt={`${name}'s ${guide.tokensTitle} page`} />
            ) : (
              <TokenPage guide={guide} />
            )}
          </BrowserFrame>
          {!isAccount && shot && (
            <p className={styles.caption}>
              {name}’s real page, captured {SHOTS_TAKEN}. Personal details are blurred.
            </p>
          )}
        </div>
        <ol className={styles.legend}>
          {legend.map((text, index) => (
            <li key={text}>
              <span className={styles.legendPin}>{index + 1}</span>
              {text}
            </li>
          ))}
        </ol>
        {!isAccount && connect && connect.permissions.length > 0 && (
          <TokenPermissions name={name} permissions={connect.permissions} />
        )}
        <div className={styles.actions}>
          {isAccount ? (
            to.allGuides && (
              <Button large onClick={to.allGuides}>
                All guides
              </Button>
            )
          ) : (
            <Button
              large
              onClick={() => {
                to.step('account');
              }}
            >
              Back
            </Button>
          )}
          <Button
            variant="accent"
            large
            disabled={!page}
            onClick={() => {
              if (page) void openExternal(page);
            }}
          >
            Open {name} {isAccount ? 'sign-up' : 'tokens'} ↗
          </Button>
          {isAccount ? (
            <Button
              variant="ghost"
              large
              onClick={() => {
                to.step('token');
              }}
            >
              I’ve made my account
            </Button>
          ) : (
            <Button variant="ghost" large onClick={to.paste}>
              I’ve copied it · paste it
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}

/**
 * The permissions to give the token, laid out as the provider's own form
 * asks for them: what it applies to, the permission, and the access level.
 */
function TokenPermissions({ name, permissions }: { name: string; permissions: TokenPermission[] }) {
  const needed = permissions.filter((permission) => !permission.optional_for);
  const optional = permissions.filter((permission) => permission.optional_for);
  return (
    <section className={styles.permissions} aria-labelledby="token-permissions">
      <h2 id="token-permissions" className={styles.permissionsTitle}>
        Permissions to give it
      </h2>
      <p className={styles.permissionsLede}>
        Add each of these on {name}’s form. Hoista asks for nothing more than it uses.
      </p>
      <table className={styles.permissionsTable}>
        <thead>
          <tr>
            <th scope="col">Applies to</th>
            <th scope="col">Permission</th>
            <th scope="col">Access</th>
            <th scope="col">Needed for</th>
          </tr>
        </thead>
        <tbody>
          {[...needed, ...optional].map((permission) => (
            <tr key={`${permission.scope}-${permission.resource}`}>
              <td>{permission.scope}</td>
              <td>{permission.resource}</td>
              <td>{permission.access}</td>
              <td className={styles.muted}>
                {permission.optional_for ? `Only for ${permission.optional_for}` : 'Always'}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
