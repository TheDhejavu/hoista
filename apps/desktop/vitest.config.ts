/** Unit tests for the adapter, shared code, the native layer and screen helpers. */
import { defineConfig } from 'vitest/config';
import { aliases } from './vite.config.ts';

export default defineConfig({
  resolve: { alias: aliases },
  test: { include: ['src/**/*.test.ts'] },
});
