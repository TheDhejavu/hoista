/**
 * The first thing people see after installing: Hoista in the middle, your
 * AI tools on the inner ring and your providers on the outer one, what
 * Hoista promises in one line, then Set up Hoista. Someone moving from
 * another computer brings their Hoista over instead, from the link beside it.
 */
import { useState, type CSSProperties } from 'react';
import { useNavigate } from 'react-router-dom';
import { ArrowRight, Check } from 'lucide-react';
import { PROVIDER_IDS } from '@shared/providers';
import { AgentLogo } from '@ui/components/AgentIcon';
import { Button } from '@ui/components/Button';
import { Mascot } from '@ui/components/Mascot';
import { ServiceIcon } from '@ui/components/ServiceIcon';
import { ImportTransferDialog } from '@ui/screens/settings/transfer/ImportTransferDialog';
import styles from './Welcome.module.css';

/** What people want to know before they start, one short line each. */
const PROMISES = [
  'Keys stay on this computer',
  'You approve anything risky',
  'Free and open source',
];

/** The AI tools on the inner ring. */
const AGENTS = ['Claude Code', 'ChatGPT', 'Codex', 'Cursor'];
/** As many providers as the outer ring holds without crowding. */
const PROVIDERS = PROVIDER_IDS.slice(0, 8);

/** Where the n-th of `count` sits on a ring, as a share of the orbit's size from its centre. */
function onRing(index: number, count: number, radius: number, turn = 0): CSSProperties {
  const angle = (2 * Math.PI * index) / count - Math.PI / 2 + turn;
  return {
    left: `${50 + radius * Math.cos(angle)}%`,
    top: `${50 + radius * Math.sin(angle)}%`,
  };
}

/** Hoista with your AI and providers circling it; decoration, so hidden from screen readers. */
function Orbit() {
  return (
    <div className={styles.orbit} aria-hidden="true">
      <span className={styles.outerTrack} />
      <span className={styles.innerTrack} />
      <div className={styles.outerRing}>
        {PROVIDERS.map((id, index) => (
          <span key={id} className={styles.slot} style={onRing(index, PROVIDERS.length, 44)}>
            <span className={styles.chip}>
              <ServiceIcon id={id} size={22} />
            </span>
          </span>
        ))}
      </div>
      <div className={styles.innerRing}>
        {AGENTS.map((name, index) => (
          <span
            key={name}
            className={styles.slot}
            style={onRing(index, AGENTS.length, 29, Math.PI / 4)}
          >
            <span className={styles.chip}>
              <AgentLogo name={name} size={20} />
            </span>
          </span>
        ))}
      </div>
      <span className={styles.hub}>
        <Mascot mood="idle" size={68} />
      </span>
    </div>
  );
}

export function WelcomeScreen() {
  const navigate = useNavigate();
  const [isImporting, setIsImporting] = useState(false);

  return (
    <div className={styles.screen}>
      <Orbit />
      <h1 className={styles.title}>Welcome to Hoista</h1>
      <p className={styles.lede}>
        Your projects on every provider, in one place. Your coding agent deploys and fixes them; you
        approve what matters.
      </p>
      <ul className={styles.promises}>
        {PROMISES.map((promise) => (
          <li key={promise}>
            <Check size={14} strokeWidth={2.4} aria-hidden="true" />
            {promise}
          </li>
        ))}
      </ul>

      <div className={styles.actions}>
        <Button
          variant="accent"
          size="hero"
          onClick={() => {
            void navigate('/setup/providers');
          }}
        >
          Set up Hoista
          <ArrowRight size={16} strokeWidth={2} aria-hidden="true" />
        </Button>
        <button
          type="button"
          className={styles.moving}
          onClick={() => {
            setIsImporting(true);
          }}
        >
          Moving from another computer? →
        </button>
      </div>
      <p className={styles.note}>Takes about two minutes. No account needed.</p>

      {isImporting && (
        <ImportTransferDialog
          onClose={() => {
            setIsImporting(false);
          }}
          onImported={() => {
            void navigate('/');
          }}
        />
      )}
    </div>
  );
}
