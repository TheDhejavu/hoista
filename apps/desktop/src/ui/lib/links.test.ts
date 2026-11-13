import { describe, expect, it } from 'vitest';
import { commitUrl, repoUrl, splitSource } from './links';

describe('links', () => {
  it('links repositories and commits on GitHub only when they are ones', () => {
    expect(repoUrl('TheDhejavu/vercel-custom-server')).toBe(
      'https://github.com/TheDhejavu/vercel-custom-server',
    );
    expect(repoUrl('not a repo')).toBeNull();
    expect(commitUrl('a/b', '03c3f26')).toBe('https://github.com/a/b/commit/03c3f26');
    expect(commitUrl('a/b', 'dpl_CRT')).toBeNull();
    expect(splitSource('a/b@03c3f26')).toEqual({ repo: 'a/b', commit: '03c3f26' });
  });
});
