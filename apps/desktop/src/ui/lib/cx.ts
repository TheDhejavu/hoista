/** Joins class names, skipping empty ones: `cx(styles.tab, isActive && styles.active)`. */
export function cx(...names: (string | false | null | undefined)[]): string {
  return names.filter(Boolean).join(' ');
}
