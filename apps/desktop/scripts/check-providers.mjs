// Fails when app code names a provider outside its entry in src/shared/providers/.
// What a provider can do comes from the engine; how it's named and drawn comes
// from its entry. Tests may name providers.
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative, sep } from 'node:path';

const root = new URL('../src/', import.meta.url).pathname;
const allowed = [`shared${sep}providers${sep}`];
const ids = [
  'cloudflare',
  'fly',
  'railway',
  'vercel',
  'neon',
  'supabase',
  'netlify',
  'render',
  'resend',
  'modal',
  'runpod',
];
const names = [
  'Cloudflare',
  'Fly.io',
  'Railway',
  'Vercel',
  'Neon',
  'Supabase',
  'Netlify',
  'Render',
  'Resend',
  'Modal',
  'Runpod',
];
// A provider id as a string ('railway'), or a provider's name anywhere in text.
const pattern = new RegExp(
  `['"\`](${ids.join('|')})['"\`]|\\b(${names.map((name) => name.replace('.', '\\.')).join('|')})\\b`,
);

function files(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return files(path);
    return /\.(ts|tsx)$/.test(name) && !/\.test\.tsx?$/.test(name) ? [path] : [];
  });
}

const found = [];
for (const path of files(root)) {
  const file = relative(root, path);
  if (allowed.some((prefix) => file.startsWith(prefix))) continue;
  const lines = readFileSync(path, 'utf8').split('\n');
  lines.forEach((line, index) => {
    const code = line.trim();
    // Comments may give a provider as an example; a line marked `provider-name-ok`
    // just before may name one for a reason given there.
    if (/^(\/\/|\/\*|\*|\{\/\*)/.test(code)) return;
    if (lines[index - 1]?.includes('provider-name-ok')) return;
    if (pattern.test(line)) found.push(`src/${file}:${String(index + 1)}: ${code}`);
  });
}

if (found.length > 0) {
  console.error(
    `A provider is named outside its entry in src/shared/providers/ (${String(found.length)}):\n${found.join('\n')}`,
  );
  process.exit(1);
}
