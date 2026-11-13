/**
 * Drawings of a provider's own sign-up and token pages inside a browser window,
 * with numbered markers on the resources to click. They show where things are,
 * not the provider's real page.
 */
import { Lock } from 'lucide-react';
import type { ReactNode } from 'react';
import { serviceName } from '@shared/providers';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { cx } from '@ui/lib/cx';
import type { ProviderGuide } from '@ui/lib/providerGuides';
import styles from './ProviderPages.module.css';

export function BrowserFrame({
  url,
  children,
  fit = false,
}: {
  url: string;
  children: ReactNode;
  /** Let the page take its content's height, for real screenshots. */
  fit?: boolean;
}) {
  return (
    <div className={styles.browser} aria-hidden="true">
      <div className={styles.chrome}>
        <span className={styles.lights}>
          <i />
          <i />
          <i />
        </span>
        <span className={styles.arrows}>‹ ›</span>
        <span className={styles.address}>
          <Lock size={11} />
          {url}
        </span>
        <span className={styles.spacer} />
      </div>
      <div className={fit ? styles.fitPage : styles.page}>{children}</div>
    </div>
  );
}

function Pin({ n, className }: { n: number; className?: string }) {
  return <span className={cx(styles.pin, className)}>{n}</span>;
}

function Bar({
  width,
  height = 7,
  light = false,
}: {
  width: number;
  height?: number;
  light?: boolean;
}) {
  return <span className={light ? styles.barLight : styles.bar} style={{ width, height }} />;
}

function ProviderHeader({ guide, tall = false }: { guide: ProviderGuide; tall?: boolean }) {
  return (
    <div className={tall ? styles.headerTall : styles.header}>
      <ServiceIcon id={guide.provider} size={tall ? 20 : 18} />
      {serviceName(guide.provider)}
      <span className={styles.headerLinks}>
        <Bar width={46} />
        <Bar width={46} />
        <Bar width={46} />
      </span>
      {!tall && <span className={styles.avatar} />}
    </div>
  );
}

export function SignupPage({ guide }: { guide: ProviderGuide }) {
  return (
    <>
      <ProviderHeader guide={guide} tall />
      <div className={styles.signup}>
        <div className={styles.signupTitle}>Create your {serviceName(guide.provider)} account</div>
        <div className={styles.signupLine}>
          <Bar width={240} height={6} light />
        </div>
        {guide.signupOptions.map((option, index) => (
          <div
            key={option}
            className={index === 0 ? styles.primaryOption : styles.option}
            style={index === 0 ? { background: guide.colour } : undefined}
          >
            {option}
            {index === 0 && <Pin n={1} className={styles.pinRight} />}
          </div>
        ))}
        <div className={styles.inbox}>
          Check your inbox for the confirmation email
          <Pin n={2} className={styles.pinInbox} />
        </div>
      </div>
    </>
  );
}

export function TokenPage({ guide }: { guide: ProviderGuide }) {
  return (
    <>
      <ProviderHeader guide={guide} />
      <div className={styles.tokenLayout}>
        <div className={styles.side}>
          {[0, 1, 2, 3, 4, 5].map((index) =>
            index === 3 ? (
              <div key={index} className={styles.sideOn}>
                {guide.tokensNav}
                <Pin n={1} className={styles.pinSide} />
              </div>
            ) : (
              <div key={index} className={styles.sideItem}>
                <Bar width={60 + ((index * 13) % 40)} />
              </div>
            ),
          )}
        </div>
        <div className={styles.tokenMain}>
          <div className={styles.tokenTitle}>{guide.tokensTitle}</div>
          <div className={styles.tokenLine}>
            <Bar width={260} height={6} light />
          </div>
          <div className={styles.form}>
            <Pin n={2} className={styles.pinForm} />
            {guide.fields.map((field, index) => (
              <div key={field.label} className={styles.formRow}>
                <span className={styles.formLabel}>{field.label}</span>
                <span className={index === 0 ? styles.fieldOn : styles.field}>{field.value}</span>
              </div>
            ))}
            <div className={styles.formFoot}>
              <span className={styles.create} style={{ background: guide.colour }}>
                {guide.createLabel}
                <Pin n={3} className={styles.pinCreate} />
              </span>
            </div>
          </div>
          <div className={styles.token}>
            <span>{guide.provider.slice(0, 2)}_••••••••••••••••••••••••7Kq2</span>
            <b>Copy</b>
          </div>
        </div>
      </div>
    </>
  );
}
