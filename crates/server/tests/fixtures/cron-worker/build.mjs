import { mkdirSync, copyFileSync } from 'node:fs';
mkdirSync('dist', { recursive: true });
copyFileSync('src/worker.js', 'dist/index.js');
copyFileSync('src/message.js', 'dist/message.js');
