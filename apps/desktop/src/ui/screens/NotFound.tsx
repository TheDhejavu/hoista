/**
 * Shown for a route that does not exist: says so plainly, then offers the
 * places people usually meant to go, and search for anything else.
 */
import { useLocation, Link } from 'react-router-dom';
import { Bot, ChevronRight, KeyRound, LayoutGrid, Rocket, Search } from 'lucide-react';
import type { ReactNode } from 'react';
import { Button } from '@ui/components/Button';
import { usePalette } from '@ui/palette/usePalette';
import styles from './NotFound.module.css';

const PLACES: { to: string; icon: ReactNode; title: string; detail: string }[] = [
  {
    to: '/',
    icon: <LayoutGrid size={16} />,
    title: 'Projects',
    detail: 'Everything you run, on every provider',
  },
  {
    to: '/deploys',
    icon: <Rocket size={16} />,
    title: 'Deploys',
    detail: 'Releases and failed deploys',
  },
  {
    to: '/agents',
    icon: <Bot size={16} />,
    title: 'Agents',
    detail: 'Who can manage your projects',
  },
  {
    to: '/accounts',
    icon: <KeyRound size={16} />,
    title: 'Accounts',
    detail: 'The providers Hoista can reach',
  },
];

export function NotFound() {
  const { pathname } = useLocation();
  const palette = usePalette();

  return (
    <div className={styles.wrap}>
      <div className={styles.card}>
        <svg className={styles.art} viewBox="0 0 120 72" aria-hidden="true">
          <path className={styles.hill} d="M0 60 Q30 38 60 52 T120 46 V72 H0Z" />
          <path className={styles.hillFront} d="M0 66 Q40 52 75 62 T120 58 V72 H0Z" />
          <circle className={styles.sun} cx="88" cy="22" r="9" />
          <path className={styles.trail} d="M14 62 C30 50 44 58 54 44" />
          <text className={styles.mark} x="58" y="40">
            ?
          </text>
        </svg>
        <h1 className={styles.title}>This screen doesn’t exist</h1>
        <p className={styles.text}>
          The link may be old, or the project it pointed to was removed. Here’s where you can go
          instead.
        </p>
        <code className={styles.path}>{pathname}</code>

        <ul className={styles.places}>
          {PLACES.map((place) => (
            <li key={place.to}>
              <Link to={place.to} className={styles.place}>
                <span className={styles.icon}>{place.icon}</span>
                <span className={styles.placeText}>
                  <span className={styles.placeTitle}>{place.title}</span>
                  <span className={styles.placeDetail}>{place.detail}</span>
                </span>
                <ChevronRight size={15} className={styles.chevron} aria-hidden="true" />
              </Link>
            </li>
          ))}
        </ul>

        <Button variant="ghost" onClick={palette.open}>
          <Search size={14} />
          Search projects and commands
          <kbd className={styles.kbd}>⌘K</kbd>
        </Button>
      </div>
    </div>
  );
}
