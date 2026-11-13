import { describe, expect, it } from 'vitest';
import { extraLines, splitCommit } from './commitMessage';

describe('splitCommit', () => {
  it('keeps the subject apart from the body', () => {
    expect(splitCommit('Fix login\n\n- Check the token\r\n- Retry once')).toEqual({
      subject: 'Fix login',
      body: '- Check the token\n- Retry once',
    });
    expect(splitCommit('One line')).toEqual({ subject: 'One line', body: '' });
    expect(splitCommit(null)).toEqual({ subject: '', body: '' });
    expect(extraLines('Fix\n\n- a\n\n- b')).toBe(2);
    expect(extraLines('Fix')).toBe(0);
  });
});
