/** Checks on links and screens that arrive from outside the page. */

/** True for http and https URLs. */
export function isExternalUrl(url: string): boolean {
  try {
    const { protocol } = new URL(url);
    return protocol === 'https:' || protocol === 'http:';
  } catch {
    return false;
  }
}

/** True for a screen the main window can open: `settings`, or a hash route such as `/projects/moodboard`. */
export function isWindowTarget(value: unknown): value is string {
  if (typeof value !== 'string') return false;
  if (value === 'settings') return true;
  return /^\/[\w\-/?=&%.]*$/.test(value) && !value.startsWith('//') && !value.includes('..');
}
