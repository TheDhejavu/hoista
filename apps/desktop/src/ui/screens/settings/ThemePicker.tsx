/**
 * Settings · General · Appearance: the theme as three pictures of the app,
 * Match system, Light and Dark, with "In use" on the current one.
 */
import type { Settings } from '@shared/api';
import { cx } from '@ui/lib/cx';
import { THEME_CHOICES } from './choices';
import styles from './ThemePicker.module.css';

type Theme = Settings['general']['theme'];

/** Match system first, as the designs show them. */
const ORDER: Theme[] = ['system', 'light', 'dark'];

export function ThemePicker({
  value,
  onChange,
}: {
  value: Theme;
  onChange: (theme: Theme) => void;
}) {
  return (
    <div className={styles.grid} role="radiogroup" aria-label="Theme">
      {ORDER.map((theme) => {
        const label = THEME_CHOICES.find((choice) => choice.value === theme)?.label ?? theme;
        const inUse = theme === value;
        return (
          <button
            key={theme}
            type="button"
            role="radio"
            aria-checked={inUse}
            className={cx(styles.card, inUse && styles.chosen)}
            onClick={() => {
              if (!inUse) onChange(theme);
            }}
          >
            <span className={styles.preview}>
              {theme === 'system' ? (
                <>
                  <Window look="light" compact />
                  <Window look="dark" compact />
                </>
              ) : (
                <Window look={theme} />
              )}
            </span>
            <span className={styles.label}>
              {label}
              {inUse && <span className={styles.inUse}>In use</span>}
            </span>
          </button>
        );
      })}
    </div>
  );
}

/** A small drawing of the app: a sidebar of lines beside four tiles. */
function Window({ look, compact = false }: { look: 'light' | 'dark'; compact?: boolean }) {
  return (
    <span className={cx(styles.window, styles[look], compact && styles.compact)}>
      <span className={styles.side}>
        <span className={styles.lineStrong} />
        <span className={styles.line} />
        <span className={styles.line} />
        {!compact && <span className={styles.line} />}
      </span>
      <span className={styles.tiles}>
        <span />
        <span />
        <span />
        <span />
      </span>
    </span>
  );
}
