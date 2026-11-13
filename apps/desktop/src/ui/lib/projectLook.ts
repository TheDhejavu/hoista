/**
 * How a project is drawn in previews: a tint, a matching ink, and the shape of
 * its page (a gallery, a landing page, a terminal…). The look is picked from the
 * project's id, so the same project always looks the same.
 */
export type PageKind =
  'gallery' | 'saas' | 'landing' | 'project' | 'api' | 'cron' | 'status' | 'chat' | 'blog';

export interface ProjectLook {
  tint: string;
  ink: string;
  kind: PageKind;
}

const PICKS: ProjectLook[] = [
  { tint: '#E9D5FF', ink: '#7C3AED', kind: 'gallery' },
  { tint: '#FEF3C7', ink: '#B45309', kind: 'landing' },
  { tint: '#DCFCE7', ink: '#15803D', kind: 'saas' },
  { tint: '#DBEAFE', ink: '#1D4ED8', kind: 'chat' },
  { tint: '#E0F2FE', ink: '#0369A1', kind: 'project' },
  { tint: '#FFE4E6', ink: '#BE123C', kind: 'blog' },
];

const TERMINAL: ProjectLook = { tint: '#F1F5F9', ink: '#334155', kind: 'cron' };

export function projectLook(project: { id: string; url: string | null }): ProjectLook {
  if (project.url === null) return TERMINAL;
  let hash = 0;
  for (const char of project.id) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  return PICKS[hash % PICKS.length] ?? TERMINAL;
}

/** Terminal-looking projects: an API or a scheduled job. */
export function isTerminal(look: ProjectLook): boolean {
  return look.kind === 'api' || look.kind === 'cron';
}

/**
 * The tint for dark mode: the same hue, dark and much less saturated, so a
 * pastel background reads as a hint of colour on the grey, not a glow.
 */
export function darkTint(hex: string): string {
  const [r, g, b] = [1, 3, 5].map((at) => parseInt(hex.slice(at, at + 2), 16) / 255) as [
    number,
    number,
    number,
  ];
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const light = (max + min) / 2;
  if (max - min < 0.04) {
    const grey = Math.round(10 + (255 - ((r + g + b) / 3) * 255) * 1.5);
    return `rgb(${grey} ${grey} ${grey})`;
  }
  const delta = max - min;
  const sat = light > 0.5 ? delta / (2 - max - min) : delta / (max + min);
  let hue =
    max === r
      ? (g - b) / delta + (g < b ? 6 : 0)
      : max === g
        ? (b - r) / delta + 2
        : (r - g) / delta + 4;
  hue *= 60;
  // Darkened yellow and orange turn khaki, so that band keeps less of its
  // colour; the rest stays a quiet tint on the grey, never a glow.
  // Yellows also lean toward amber: dark amber reads warm, dark yellow green.
  const muddy = hue >= 28 && hue <= 70;
  const keep = Math.min(sat, muddy ? 0.26 : 0.34);
  const shown = muddy ? 30 : hue;
  return `hsl(${shown.toFixed(0)} ${(keep * 100).toFixed(0)}% ${muddy ? 14 : 15}%)`;
}
