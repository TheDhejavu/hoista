/**
 * `hoist` commands for what's on screen, ready to paste in a terminal: the
 * same thing the app shows, from the command line. Words are quoted for the
 * shell only when they need it.
 */
import { useSettings } from '@ui/api/settings';
import { useCurrentEnvironment } from '@ui/api/environments';

/** A word as the shell reads it: as it is when it's plain, otherwise in single quotes. */
export function shellWord(word: string): string {
  if (word !== '' && /^[A-Za-z0-9_@%+=:,./~-]+$/.test(word) && !word.startsWith('~')) return word;
  return `'${word.replace(/'/g, `'\\''`)}'`;
}

/** `hoist` and its words as one line. `program` is how to run hoist. */
export function hoistLine(program: string, words: string[]): string {
  return [program, ...words.map(shellWord)].join(' ');
}

/**
 * Builds `hoist` commands: `hoist` when it's on the PATH, its full path when
 * it isn't, and `-e NAME` when there are several environments, so the command
 * reaches the same project the window shows.
 */
export function useHoistCommand(): (words: string[]) => string {
  const cli = useSettings().data?.cli;
  const { environment, environments } = useCurrentEnvironment();
  const program = cli && !cli.installed && cli.path ? shellWord(cli.path) : 'hoist';
  const scope = environments.length > 1 && environment ? ['-e', environment.name] : [];
  return (words) => hoistLine(program, [...words, ...scope]);
}
