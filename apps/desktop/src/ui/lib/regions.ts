/**
 * Region names people read, for the codes providers' APIs return. Railway gives
 * both airport-style codes (`iad`) and zone ids (`us-east4-eqdc4a`); both
 * read as the place. Unknown codes are shown as they are.
 */
const PLACES: [RegExp, string][] = [
  [/^(iad|us-east)/i, 'US East · Virginia'],
  [/^(sjc|sfo|us-west)/i, 'US West · California'],
  [/^(ams|europe-west4|eu-west)/i, 'EU West · Amsterdam'],
  [/^(sin|asia-southeast)/i, 'Southeast Asia · Singapore'],
  [/^(lhr|europe-west2)/i, 'UK · London'],
  [/^(fra|europe-west3)/i, 'EU Central · Frankfurt'],
  [/^(syd|australia-southeast)/i, 'Australia · Sydney'],
];

export function regionName(code: string): string {
  return PLACES.find(([pattern]) => pattern.test(code))?.[1] ?? code;
}
