import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';
const source = await readFile(new URL('./sourceCitations.ts', import.meta.url), 'utf8');
const output = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const { citationHref, parseCitation, citationOffset, parseComposerCitations } = await import(`data:text/javascript;base64,${Buffer.from(output).toString('base64')}`);
test('source references survive persistence and special characters', () => {
  const citation = { session: 'chat-1', turn: 'turn-32', quote: 'a & b (c)\n日本語', before: '[first]', after: '[last]' };
  assert.deepEqual(parseCitation(citationHref(citation)), citation);
  assert.ok(!/[()]/.test(citationHref(citation)));
  assert.equal(parseCitation('#mira-citation=%broken'), null);
  assert.equal(parseCitation('#mira-citation=%7B%7D'), null);
});
test('repeated passages resolve by surrounding context and never substitute a changed passage', () => {
  const citation = { session: 'a', turn: 'turn-2', quote: 'same', before: 'second ', after: ' end' };
  assert.equal(citationOffset('first same text, second same end', citation), 24);
  assert.equal(citationOffset('first same text, second changed end', citation), -1);
});

test('composer references have readable labels and retain their full canonical payload', () => {
  const citation = { session: 'chat-1', turn: 'turn-4', quote: 'approved', before: 'Mira (native): ', after: '\n日本語' };
  const raw = `[reply 5](${citationHref(citation)})`;
  const text = `Regarding ${raw}:\n> approved\n\n@skill:review explain this`;
  const segments = parseComposerCitations(text);
  assert.deepEqual(segments[1], { kind: 'citation', label: 'reply 5', raw, citation });
  assert.equal(segments.map(segment => segment.kind === 'citation' ? segment.raw : segment.text).join(''), text);
  assert.equal(segments.map(segment => segment.kind === 'citation' ? segment.label : segment.text).join(''), 'Regarding reply 5:\n> approved\n\n@skill:review explain this');
});

test('multiple composer references coexist with malformed links and ordinary Markdown', () => {
  const citation = { session: 'a', turn: 'turn-0', quote: 'quote', before: '', after: '' };
  const raw = `[reply 1](${citationHref(citation)})`;
  const text = `[broken](#mira-citation=%broken) ${raw} [docs](https://example.com) ${raw}`;
  const segments = parseComposerCitations(text);
  assert.equal(segments.filter(segment => segment.kind === 'citation').length, 2);
  assert.equal(segments.map(segment => segment.kind === 'citation' ? segment.raw : segment.text).join(''), text);
  assert.deepEqual(parseComposerCitations('plain text'), [{ kind: 'text', text: 'plain text' }]);
  assert.deepEqual(parseComposerCitations(''), []);
});
