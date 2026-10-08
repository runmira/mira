#!/usr/bin/env node
// Bundle size budget for the web UI (issue #72).
//
// Sums the gzipped size of the JS a first page load fetches: the `index.html`
// entry plus every chunk it imports statically, per Vite's build manifest.
// Lazy chunks (settings, diffs, mermaid, grammars, …) don't count. Exits
// non-zero past the budget so CI fails when the initial bundle grows.
//
//   npm run build && npm run size
import { readFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';
import path from 'node:path';

const BUDGET_KB = Number(process.env.BUNDLE_BUDGET_KB ?? 400);
const dist = path.resolve(import.meta.dirname, '../dist');
const manifest = JSON.parse(readFileSync(path.join(dist, '.vite/manifest.json'), 'utf8'));

const seen = new Set();
const files = [];
(function walk(key) {
  if (seen.has(key)) return;
  seen.add(key);
  const chunk = manifest[key];
  if (chunk.file.endsWith('.js')) files.push(chunk.file);
  for (const dep of chunk.imports ?? []) walk(dep);
})('index.html');

let total = 0;
for (const file of files) {
  const kb = gzipSync(readFileSync(path.join(dist, file))).length / 1024;
  total += kb;
  console.log(`${kb.toFixed(1).padStart(8)} kB  ${file}`);
}
const ok = total <= BUDGET_KB;
console.log(`${total.toFixed(1).padStart(8)} kB  initial JS, gzipped (budget ${BUDGET_KB} kB) ${ok ? '✓' : '✗ over budget'}`);
process.exit(ok ? 0 : 1);
