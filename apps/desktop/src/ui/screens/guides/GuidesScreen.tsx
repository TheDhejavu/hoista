/**
 * Provider guides: one card per provider with its guides, making an account and
 * connecting Hoista. Providers that offer it connect on their own page; every
 * provider also has the token guide, with pictures of its pages.
 */
import { serviceName } from '@shared/providers';
import { useConnectGuide } from '@ui/api/accounts';
import { ButtonLink } from '@ui/components/Button';
import { Page } from '@ui/components/Page';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { PROVIDER_GUIDES, type ProviderGuide } from '@ui/lib/providerGuides';
import styles from './GuidesScreen.module.css';

export function GuidesScreen() {
  return (
    <Page>
      <div>
        <h1 className={styles.title}>Provider guides</h1>
        <p className={styles.lede}>
          Step-by-step pictures of each provider’s own pages, so you know exactly where to click.
          Where a provider offers it, Connect takes you to its page to approve Hoista, with no token
          to copy.
        </p>
      </div>
      <div className={styles.grid}>
        {PROVIDER_GUIDES.map((guide) => (
          <GuideCard key={guide.provider} guide={guide} />
        ))}
      </div>
    </Page>
  );
}

function GuideCard({ guide }: { guide: ProviderGuide }) {
  const oneClick = Boolean(useConnectGuide(guide.provider).data?.sign_in);

  return (
    <div className={styles.card}>
      <div className={styles.head}>
        <span className={styles.logo}>
          <ServiceIcon id={guide.provider} size={19} />
        </span>
        <span>
          <b>{serviceName(guide.provider)}</b>
          <span className={styles.muted}>{guide.usedFor}</span>
        </span>
      </div>
      <div className={styles.plan}>{guide.plan}</div>
      <div className={styles.actions}>
        <ButtonLink to={`/guides/${guide.provider}/account`}>Make an account</ButtonLink>
        {oneClick ? (
          <ButtonLink to={`/accounts/connect/${guide.provider}`} variant="accent">
            Connect
          </ButtonLink>
        ) : (
          <ButtonLink to={`/guides/${guide.provider}/token`} variant="accent">
            Create a token
          </ButtonLink>
        )}
      </div>
    </div>
  );
}
