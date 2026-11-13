/**
 * Hoista's mascot, drawn inline so it takes the brand colour and its face
 * shows whatever is behind it. Each mood has its own pose and loop; `still`
 * turns the loop off. The logo, favicon and app icon use the same shapes.
 */
import { useId } from 'react';
import { cx } from '@ui/lib/cx';
import styles from './Mascot.module.css';

export type MascotMood =
  | 'logo' // standing with the bar, no motion
  | 'mark' // the small faceless mark, for 24 px and under
  | 'idle' // breathes and blinks
  | 'loading' // hops
  | 'deploying' // lifted on a crane hook
  | 'done' // cheers
  | 'empty' // peeks over a line
  | 'hello' // waves
  | 'asleep' // eyes shut, z's drift up
  | 'failed'; // drops the bar

const ARMS = (
  <>
    <rect x="8" y="24" width="22" height="58" rx="11" />
    <rect x="70" y="24" width="22" height="58" rx="11" />
  </>
);
const TORSO = (
  <>
    <circle cx="50" cy="53" r="22" />
    <rect x="8" y="53" width="84" height="25" rx="6" />
  </>
);
const LEGS = (
  <>
    <rect x="8" y="58" width="30" height="36" rx="15" />
    <rect x="62" y="58" width="30" height="36" rx="15" />
  </>
);
const BAR = <rect x="25" y="6" width="50" height="12" rx="6" />;
const OPEN_EYES = (
  <>
    <rect x="41" y="43" width="6.5" height="11" rx="3.25" transform="rotate(-10 44 48)" />
    <rect x="52.5" y="41" width="6.5" height="11" rx="3.25" transform="rotate(-10 56 46)" />
  </>
);
const LINE_EYES = {
  fill: 'none',
  stroke: '#000',
  strokeWidth: 3.5,
  strokeLinecap: 'round',
} as const;

export function Mascot({
  mood = 'logo',
  size = 48,
  still = false,
  label,
}: {
  mood?: MascotMood;
  size?: number;
  still?: boolean;
  /** Read out by screen readers; leave it out when the mascot is decoration. */
  label?: string;
}) {
  const id = useId().replace(/:/g, '');
  const mask = `${id}-face`;
  const ink = 'currentColor';
  const eyes =
    mood === 'done' ? (
      <path d="M40 50Q44 43 48 50M52 48Q56 41 60 48" {...LINE_EYES} />
    ) : mood === 'asleep' ? (
      <path d="M40 52H47M53 50H60" {...LINE_EYES} />
    ) : mood === 'failed' ? (
      <path d="M40 50L47 46M53 46L60 50" {...LINE_EYES} />
    ) : (
      <g className={styles.eyes}>{OPEN_EYES}</g>
    );
  const face = (
    <mask id={mask} maskUnits="userSpaceOnUse" x="-20" y="-60" width="140" height="180">
      <rect x="-20" y="-60" width="140" height="180" fill="#fff" />
      <g fill="#000">{eyes}</g>
    </mask>
  );

  let body;
  switch (mood) {
    case 'mark':
      body = (
        <g fill={ink}>
          <rect x="22" y="4" width="56" height="14" rx="7" />
          <rect x="6" y="24" width="26" height="70" rx="13" />
          <rect x="68" y="24" width="26" height="70" rx="13" />
          <circle cx="50" cy="52" r="23" />
          <rect x="6" y="52" width="88" height="24" rx="6" />
        </g>
      );
      break;
    case 'deploying':
      body = (
        <g className={styles.swing}>
          <g className={styles.rise}>
            <rect x="47.5" y="-60" width="5" height="69" rx="2" fill={ink} />
            <path
              d="M50 8V15A7 7 0 1 1 43 22"
              fill="none"
              stroke={ink}
              strokeWidth="5"
              strokeLinecap="round"
            />
            <g fill={ink} mask={`url(#${mask})`}>
              {ARMS}
              {TORSO}
              {LEGS}
            </g>
          </g>
        </g>
      );
      break;
    case 'done':
      body = (
        <g className={styles.jump}>
          <path
            className={styles.sparks}
            d="M10 15L6 8M19 12V3M28 15L32 8M72 15L68 8M81 12V3M90 15L94 8"
            fill="none"
            stroke={ink}
            strokeWidth="3"
            strokeLinecap="round"
          />
          <g fill={ink} mask={`url(#${mask})`}>
            {ARMS}
            {TORSO}
            {LEGS}
          </g>
        </g>
      );
      break;
    case 'empty':
      body = (
        <>
          <clipPath id={`${id}-line`}>
            <rect x="-20" y="-60" width="140" height="130" />
          </clipPath>
          <g clipPath={`url(#${id}-line)`}>
            <g className={styles.peek} fill={ink} mask={`url(#${mask})`}>
              {ARMS}
              {TORSO}
            </g>
          </g>
          <rect x="-4" y="72" width="108" height="7" rx="3.5" fill={ink} />
        </>
      );
      break;
    case 'hello':
      body = (
        <g fill={ink} mask={`url(#${mask})`}>
          <rect className={styles.wave} x="70" y="24" width="22" height="58" rx="11" />
          <rect x="8" y="24" width="22" height="58" rx="11" />
          {TORSO}
          {LEGS}
        </g>
      );
      break;
    case 'asleep':
      body = (
        <>
          <g className={styles.breathe} fill={ink} mask={`url(#${mask})`}>
            <rect x="25" y="20" width="50" height="10" rx="5" />
            <rect x="8" y="27" width="22" height="55" rx="11" />
            <rect x="70" y="27" width="22" height="55" rx="11" />
            <circle cx="50" cy="55" r="22" />
            <rect x="8" y="55" width="84" height="23" rx="6" />
            {LEGS}
          </g>
          <path
            className={styles.z}
            d="M80 2H88L80 10H88"
            fill="none"
            stroke={ink}
            strokeWidth="2.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
          <path
            className={cx(styles.z, styles.later)}
            d="M80 2H86L80 8H86"
            fill="none"
            stroke={ink}
            strokeWidth="2.2"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </>
      );
      break;
    case 'failed':
      body = (
        <>
          <rect className={styles.drop} x="25" y="6" width="50" height="12" rx="6" fill={ink} />
          <g className={styles.shake} fill={ink} mask={`url(#${mask})`}>
            {ARMS}
            {TORSO}
            {LEGS}
          </g>
        </>
      );
      break;
    default: {
      // logo, idle and loading: the standing pose with the bar.
      const motion = mood === 'loading' ? styles.hop : mood === 'idle' ? styles.breathe : undefined;
      body = (
        <g className={motion} fill={ink} mask={`url(#${mask})`}>
          {BAR}
          {ARMS}
          {TORSO}
          {LEGS}
        </g>
      );
    }
  }

  // Room above and below for the hook, the hop and the sparks.
  const roomy = mood === 'deploying' || mood === 'loading' || mood === 'done';
  return (
    <svg
      className={cx(styles.mascot, (still || mood === 'logo' || mood === 'mark') && styles.still)}
      data-mood={mood}
      width={size}
      height={size}
      viewBox={roomy ? '-10 -20 120 120' : '0 0 100 100'}
      role={label ? 'img' : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
    >
      {mood !== 'mark' && <defs>{face}</defs>}
      {body}
    </svg>
  );
}
