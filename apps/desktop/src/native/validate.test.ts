import { describe, expect, it } from 'vitest';
import { isExternalUrl, isWindowTarget } from './validate';

describe('isExternalUrl', () => {
  it('allows only http and https', () => {
    expect(isExternalUrl('https://hoista.dev')).toBe(true);
    expect(isExternalUrl('file:///etc/passwd')).toBe(false);
    expect(isExternalUrl('javascript:alert(1)')).toBe(false);
  });
});

describe('isWindowTarget', () => {
  it('accepts settings and project routes', () => {
    expect(isWindowTarget('settings')).toBe(true);
    expect(isWindowTarget('/projects/moodboard')).toBe(true);
    expect(isWindowTarget('/reconnect/cloudflare?agent=Claude%20Code&project=moodboard')).toBe(
      true,
    );
  });

  it('refuses anything else', () => {
    expect(isWindowTarget('https://example.com')).toBe(false);
    expect(isWindowTarget('//evil.example')).toBe(false);
    expect(isWindowTarget('/projects/../../etc')).toBe(false);
    expect(isWindowTarget('javascript:alert(1)')).toBe(false);
    expect(isWindowTarget(42)).toBe(false);
  });
});
