/** Builds the page every window loads. */
import { resolve } from 'node:path';
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

const src = (folder: string): string => resolve(import.meta.dirname, 'src', folder);

export const aliases = {
  '@adapter': src('adapter'),
  '@engine-api': resolve(import.meta.dirname, '../../docs/backend/api/types.ts'),
  '@native': src('native'),
  '@shared': src('shared'),
  '@ui': src('ui'),
};

export default defineConfig({
  resolve: { alias: aliases },
  plugins: [react()],
  clearScreen: false,
  server: { port: 5174, strictPort: true },
  build: {
    outDir: 'dist',
    target: 'es2022',
  },
});
