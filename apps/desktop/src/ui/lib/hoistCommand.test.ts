import { describe, expect, it } from 'vitest';
import { hoistLine, shellWord } from './hoistCommand';

describe('shellWord', () => {
  it('leaves plain words alone', () => {
    expect(shellWord('sentinel-edge')).toBe('sentinel-edge');
    expect(shellWord('provider~res~dep')).toBe('provider~res~dep');
  });

  it('quotes words the shell would split or expand', () => {
    expect(shellWord('my app')).toBe(`'my app'`);
    expect(shellWord(`Sam's app`)).toBe(`'Sam'\\''s app'`);
    expect(shellWord('~home')).toBe(`'~home'`);
    expect(shellWord('$HOME')).toBe(`'$HOME'`);
    expect(shellWord('')).toBe(`''`);
  });
});

describe('hoistLine', () => {
  it('joins the program and its words', () => {
    expect(hoistLine('hoist', ['logs', 'my app', '--follow'])).toBe(`hoist logs 'my app' --follow`);
  });
});
