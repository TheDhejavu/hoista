import { describe, expect, it } from 'vitest';
import { projectUrl } from './projects';

describe('a project’s address', () => {
  it('keeps the person’s own domain when a new service brings a provider address', () => {
    expect(
      projectUrl([
        { url: 'https://mcp-test-production-bbd4.up.railway.app' },
        { url: 'https://api.sam.dev' },
        { url: null },
      ]),
    ).toBe('https://api.sam.dev');
  });

  it('falls back to the first address there is', () => {
    expect(projectUrl([{ url: null }, { url: 'https://web.up.railway.app' }])).toBe(
      'https://web.up.railway.app',
    );
    expect(projectUrl([{ url: null }])).toBeNull();
  });

  it('shows the website before an API or a GPU endpoint', () => {
    expect(
      projectUrl([
        { url: 'https://sam--analyze.modal.run', kind: 'service' },
        { url: 'https://sentinel-dashboard.vercel.app', kind: 'web' },
        { url: 'https://sentinel-api.fly.dev', kind: 'service' },
      ]),
    ).toBe('https://sentinel-dashboard.vercel.app');
  });
});
