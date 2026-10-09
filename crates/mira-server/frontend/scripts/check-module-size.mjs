import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
const root = path.resolve(import.meta.dirname, '../src');
const limit = 1000;
const oversized = [];
function visit(directory) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const file = path.join(directory, entry.name);
    if (entry.isDirectory()) visit(file);
    else if (/\.(ts|tsx|css)$/.test(file)) {
      const lines = readFileSync(file, 'utf8').trimEnd().split('\n').length;
      if (lines > limit) oversized.push(`${path.relative(root, file)}: ${lines} lines`);
    }
  }
}
visit(root);
if (oversized.length) {
  console.error(`Frontend modules must stay within ${limit} lines:\n${oversized.join('\n')}`);
  process.exitCode = 1;
} else console.log(`All frontend modules stay within ${limit} lines.`);
