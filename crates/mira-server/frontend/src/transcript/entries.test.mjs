import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

const { outputText } = ts.transpileModule(
  readFileSync(new URL('./entries.ts', import.meta.url), 'utf8'),
  { compilerOptions: { module: ts.ModuleKind.ESNext } },
);
const { stripHookContext } = await import(
  `data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`
);

test('stripHookContext strips generated hook-context blocks', () => {
  const stored = 'fix it\n\n<hook-context>\nbe educational\n</hook-context>';
  assert.equal(stripHookContext(stored), 'fix it');
});

test('stripHookContext strips generated memory-context blocks', () => {
  const stored = 'ok\n\n<memory-context>\nuses pnpm\n</memory-context>';
  assert.equal(stripHookContext(stored), 'ok');
});

test('stripHookContext strips combined hook and memory context blocks', () => {
  const stored =
    'fix it\n\n<hook-context>\nbe kind\n</hook-context>\n\n<memory-context>\nuses pnpm\n</memory-context>';
  assert.equal(stripHookContext(stored), 'fix it');
});

test('stripHookContext preserves plain text without context blocks', () => {
  assert.equal(stripHookContext('plain'), 'plain');
  assert.equal(stripHookContext(''), '');
  assert.equal(stripHookContext(null), null);
  assert.equal(stripHookContext(undefined), undefined);
});

test('stripHookContext preserves user-authored markers without \\n\\n prefix', () => {
  // User typing a literal marker (no \n\n before it) should be preserved.
  assert.equal(
    stripHookContext('Check <hook-context> usage in the docs'),
    'Check <hook-context> usage in the docs',
  );
  assert.equal(
    stripHookContext('See the <memory-context> tag here'),
    'See the <memory-context> tag here',
  );
});

test('stripHookContext preserves user-authored markers with single newline prefix', () => {
  // Only single \n before (not \n\n) should be preserved.
  assert.equal(
    stripHookContext('Line 1\n<memory-context> is special'),
    'Line 1\n<memory-context> is special',
  );
  assert.equal(
    stripHookContext('Line 1\n<hook-context> marker'),
    'Line 1\n<hook-context> marker',
  );
});

test('stripHookContext preserves user-authored markers without \\n after opening tag', () => {
  // Marker without \n after opening tag (inline marker) should be preserved.
  assert.equal(
    stripHookContext('See\n\n<hook-context>inline marker</hook-context>'),
    'See\n\n<hook-context>inline marker</hook-context>',
  );
  assert.equal(
    stripHookContext('Example\n\n<memory-context>inline</memory-context>'),
    'Example\n\n<memory-context>inline</memory-context>',
  );
});

test('stripHookContext preserves user text after a literal marker pattern', () => {
  // User content that happens to include the marker text, followed by more text.
  assert.equal(
    stripHookContext('Before <memory-context>\nAfter the marker'),
    'Before <memory-context>\nAfter the marker',
  );
  assert.equal(
    stripHookContext('I wrote <hook-context> and then continued writing'),
    'I wrote <hook-context> and then continued writing',
  );
});

test('stripHookContext handles edge cases with leading content before generated block', () => {
  // Real user prompt with generated context appended at end.
  const prompt = 'Please fix the bug in src/main.ts\n\n<hook-context>\nproject uses ESLint\n</hook-context>';
  assert.equal(stripHookContext(prompt), 'Please fix the bug in src/main.ts');

  // Multi-line user prompt.
  const multiLine = 'Line 1\nLine 2\nLine 3\n\n<memory-context>\nmemory data\n</memory-context>';
  assert.equal(stripHookContext(multiLine), 'Line 1\nLine 2\nLine 3');
});
