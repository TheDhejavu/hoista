// Run only checked-in provider-free fixtures through the locked local Workers runtime.
import { Miniflare, convertV4MiniflareOptions } from 'miniflare';
import assert from 'node:assert/strict';
const runtime = new Miniflare(convertV4MiniflareOptions({
  modules: true, scriptPath: process.argv[2],
  compatibilityDate: '2026-11-01', compatibilityFlags: ['nodejs_compat'],
  cf: false, host: '127.0.0.1', port: 0,
}));
let deadline;
try {
  await Promise.race([
    (async () => {
      const response = await runtime.dispatchFetch('http://fixture.local/');
      const html = await response.text();
      assert.equal(response.status, 200);
      assert.match(html, /Hoista SSR fixture/);
      assert.match(html, /hoista-server-only-fixture/);
      assert.match(html, /<script/);
    })(),
    new Promise((_, reject) => { deadline = setTimeout(() => reject(new Error('Runtime fixture timed out')), 30000); }),
  ]);
} finally {
  clearTimeout(deadline);
  await runtime.dispose();
}
