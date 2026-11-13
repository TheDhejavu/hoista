/**
 * Logos in small rounded tiles for whoever did something: an AI tool such as
 * Claude Code or Cursor, a provider, or Hoista itself. Tools Hoista has no logo
 * for show their initials. An optional dot says how the last call went.
 */
import type { ActivityItem } from '@shared/api';
import { agentLogo } from '@shared/agentLogos';
import { useProfile } from '@ui/api/accounts';
import { SERVICES } from '@shared/providers';
import { cx } from '@ui/lib/cx';
import { Dot, type Tone } from './Dot';
import { LogoMark } from './icons';
import { ServiceIcon } from './ServiceIcon';
import styles from './AgentIcon.module.css';

export function AgentIcon({
  name,
  tone,
  small = false,
  person = false,
}: {
  name: string;
  tone?: Tone;
  small?: boolean;
  /** You, working in Hoista itself: shows Hoista's mark. */
  person?: boolean;
}) {
  const logo = person ? undefined : agentLogo(name);

  return (
    <span className={cx(styles.tile, small && styles.small)} aria-hidden="true">
      {logo ? (
        <AgentLogo name={name} className={styles.logo} />
      ) : person ? (
        <LogoMark size={small ? 11 : 14} />
      ) : (
        <span className={styles.initials}>{initials(name)}</span>
      )}
      {tone && (
        <span className={styles.dot}>
          <Dot tone={tone} small />
        </span>
      )}
    </span>
  );
}

/**
 * An AI tool's logo on its own, without a tile, in its brand colour (or the
 * text colour). Draws nothing for a tool Hoista has no logo for.
 */
export function AgentLogo({
  name,
  size,
  className,
}: {
  name: string;
  size?: number;
  className?: string;
}) {
  const logo = agentLogo(name);
  if (!logo) return null;
  return (
    <svg className={className} width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
      <path
        d={logo.path}
        fill={logo.colour ?? 'currentColor'}
        fillRule={logo.evenOdd ? 'evenodd' : undefined}
        clipRule={logo.evenOdd ? 'evenodd' : undefined}
      />
    </svg>
  );
}

/**
 * Whoever did something in a project's activity: an agent's logo, the provider's
 * logo (Neon, Fly.io…), Hoista's mark, or your initial.
 */
export function ActorIcon({ actor }: { actor: ActivityItem['actor'] }) {
  if (actor.kind === 'agent') return <AgentIcon name={actor.name} small />;

  if (actor.kind === 'person') return <PersonAvatar />;

  const service = Object.entries(SERVICES).find(([, info]) => info.name === actor.name)?.[0];
  if (!service) {
    // Hoista itself: its mark in white on a black tile.
    return (
      <span className={cx(styles.tile, styles.small, styles.hoista)} aria-hidden="true">
        <LogoMark size={13} />
      </span>
    );
  }
  return (
    <span className={cx(styles.tile, styles.small)} aria-hidden="true">
      <ServiceIcon id={service} size={13} />
    </span>
  );
}

/** You: your initials in a round black badge, as in the profile menu. */
export function PersonAvatar({ size = 22 }: { size?: number }) {
  const profile = useProfile();
  return (
    <span
      className={styles.person}
      style={{ width: size, height: size, fontSize: Math.round(size * 0.38) }}
      title="You"
      aria-hidden="true"
    >
      {profile.data?.initials ?? ''}
    </span>
  );
}

/** `CC` for Claude Code, `Ch` for ChatGPT, so tools that share a first letter stay apart. */
function initials(name: string): string {
  const words = name.split(/\s+/).filter(Boolean);
  if (words.length > 1)
    return words
      .map((word) => word.charAt(0))
      .join('')
      .slice(0, 2);
  return name.slice(0, 2);
}
