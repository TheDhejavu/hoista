import { describe, expect, it } from 'vitest';
import { pushChoice } from './PushOffer';

const counts = (projects: number, accounts: number) => ({ projects, accounts });

describe('pushChoice', () => {
  it('offers a push only into an empty remote, when this Mac has something', () => {
    expect(pushChoice({ remote: counts(0, 0), here: counts(3, 2) })).toBe('offer');
    expect(pushChoice({ remote: counts(0, 0), here: counts(0, 1) })).toBe('offer');
  });
  it('never merges into a remote that already holds anything', () => {
    expect(pushChoice({ remote: counts(1, 0), here: counts(3, 2) })).toBe('taken');
    expect(pushChoice({ remote: counts(0, 1), here: counts(3, 2) })).toBe('taken');
  });
  it('has nothing to offer when this Mac is empty', () => {
    expect(pushChoice({ remote: counts(0, 0), here: counts(0, 0) })).toBe('nothing');
  });
});
