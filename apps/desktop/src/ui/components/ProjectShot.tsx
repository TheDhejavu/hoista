/**
 * A drawing of a project's page for the Projects grid: its own tint around a little
 * browser page in the project's shape (a gallery, a landing page, a chat, a
 * terminal…), so each card looks like the real thing without a screenshot.
 * A terminal is drawn as blank lines, so nothing on the card reads like a log
 * the project never wrote.
 */
import type { ReactNode } from 'react';
import type { ProjectSummary } from '@shared/api';
import { projectLook, type ProjectLook } from '@ui/lib/projectLook';
import { tintVars } from './ProjectPreviewIcon';
import styles from './ProjectShot.module.css';

export function ProjectShot({
  project,
  height,
  badge,
}: {
  project: Pick<ProjectSummary, 'id' | 'url'>;
  height: number;
  /** Shown in the top-left corner, such as "Needs a key". */
  badge?: ReactNode;
}) {
  const look = projectLook(project);

  return (
    <div className={styles.shot} style={{ height, ...tintVars(look.tint) }} aria-hidden="true">
      {look.kind === 'api' || look.kind === 'cron' ? (
        <Terminal />
      ) : (
        <div className={styles.page}>
          <Nav ink={look.ink} />
          <Body look={look} />
        </div>
      )}
      {badge && <div className={styles.badge}>{badge}</div>}
    </div>
  );
}

/** Widths of the blank terminal lines, as percentages. */
const TERMINAL_LINES = [72, 58, 80, 46];

function Terminal() {
  return (
    <div className={styles.terminal}>
      <div className={styles.prompt}>$</div>
      {TERMINAL_LINES.map((width) => (
        <span key={width} className={styles.termLine} style={{ width: `${String(width)}%` }} />
      ))}
    </div>
  );
}

function Nav({ ink }: { ink: string }) {
  return (
    <div className={styles.nav}>
      <span className={styles.logo} style={{ background: ink }} />
      <span className={styles.ln} style={{ width: 40, height: 5 }} />
      <span className={styles.navLinks}>
        <span className={styles.ln} style={{ width: 18, height: 4 }} />
        <span className={styles.ln} style={{ width: 18, height: 4 }} />
      </span>
    </div>
  );
}

const TINT = 'var(--shot-tint)';
const FILL = 'var(--shot-fill)';
const GALLERY: [number, string][] = [
  [46, TINT],
  [30, FILL],
  [38, TINT],
  [30, FILL],
  [46, FILL],
  [34, TINT],
  [40, FILL],
  [52, TINT],
  [36, FILL],
  [52, FILL],
  [40, TINT],
  [46, FILL],
  [36, TINT],
  [44, FILL],
  [40, TINT],
];

function Body({ look }: { look: ProjectLook }) {
  switch (look.kind) {
    case 'gallery':
      return (
        <div className={styles.gallery}>
          {GALLERY.map(([height, background], index) => (
            <div key={index} style={{ height, background, borderRadius: 6 }} />
          ))}
        </div>
      );
    case 'landing':
      return (
        <div className={styles.landing}>
          <span
            className={styles.ln}
            style={{ width: '70%', height: 10, background: look.ink, margin: '0 auto 8px' }}
          />
          <span className={styles.ln} style={{ width: '52%', margin: '0 auto 14px' }} />
          <div className={styles.ctaRow}>
            <span className={styles.cta} style={{ width: 90, background: FILL }} />
            <span className={styles.cta} style={{ width: 54, background: look.ink }} />
          </div>
          <div className={styles.hero} />
        </div>
      );
    case 'status':
      return (
        <div className={styles.status}>
          <div className={styles.statusHead}>
            <span className={styles.dot} />
            <span className={styles.ln} style={{ width: 110, background: look.ink }} />
          </div>
          {[0, 1].map((row) => (
            <div key={row} className={styles.statusRow}>
              <span className={styles.ln} style={{ width: 60, marginBottom: 6 }} />
              <div className={styles.bars}>
                {Array.from({ length: 30 }, (_, index) => (
                  <i key={index} className={index === 17 ? styles.barWarn : styles.barGood} />
                ))}
              </div>
            </div>
          ))}
        </div>
      );
    case 'chat':
      return (
        <div className={styles.chat}>
          <span
            className={styles.bubble}
            style={{ alignSelf: 'flex-start', width: '58%', background: FILL }}
          />
          <span
            className={styles.bubble}
            style={{ alignSelf: 'flex-end', width: '46%', background: look.ink }}
          />
          <span
            className={styles.bubble}
            style={{ alignSelf: 'flex-start', width: '64%', background: FILL }}
          />
          <span
            className={styles.bubble}
            style={{ alignSelf: 'flex-end', width: '30%', background: TINT }}
          />
        </div>
      );
    case 'blog':
      return (
        <div className={styles.blog}>
          <span
            className={styles.ln}
            style={{ width: '40%', height: 11, background: 'var(--text)', marginBottom: 14 }}
          />
          {[0, 1, 2].map((row) => (
            <div key={row}>
              <span className={styles.ln} style={{ width: '92%', marginBottom: 7 }} />
              <span className={styles.ln} style={{ width: '74%', marginBottom: 14 }} />
            </div>
          ))}
        </div>
      );
    default:
      return (
        <div className={styles.saas}>
          <div className={styles.saasSide}>
            {[80, 60, 70, 50].map((width) => (
              <span
                key={width}
                className={styles.ln}
                style={{ width: `${width}%`, marginBottom: 9 }}
              />
            ))}
          </div>
          <div className={styles.saasMain}>
            <div className={styles.saasCards}>
              {[TINT, FILL, FILL].map((background, index) => (
                <span key={index} style={{ background }} />
              ))}
            </div>
            {[0, 1, 2, 3].map((row) => (
              <span key={row} className={styles.ln} style={{ marginBottom: 8 }} />
            ))}
          </div>
        </div>
      );
  }
}
