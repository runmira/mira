import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';

const source = readFileSync(new URL('../src/lib/sessionRefs.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ESNext },
});
const { linkifySessionRefs, parseSessionRef, shortSessionId } = await import(
  `data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`
);

const ID = 'sess_a41504d51dd44bfca7e51968e0e88f24';

test('bare session id becomes a session link', () => {
  assert.equal(linkifySessionRefs(`see ${ID} for context`), `see [${ID}](#session-${ID}) for context`);
});

test('text without sess_ is untouched', () => {
  assert.equal(linkifySessionRefs('nothing here'), 'nothing here');
});

test('code spans are off-limits', () => {
  assert.equal(linkifySessionRefs(`\`${ID}\``), `\`${ID}\``);
  assert.equal(
    linkifySessionRefs(`\`\`\`\n${ID}\n\`\`\``),
    `\`\`\`\n${ID}\n\`\`\``,
  );
});

test('already-linked sessions are not double-linked', () => {
  const once = linkifySessionRefs(ID);
  assert.equal(linkifySessionRefs(once), once);
});

test('short ids and non-session hrefs are ignored', () => {
  assert.equal(linkifySessionRefs('sess_abc123'), 'sess_abc123');
  assert.equal(parseSessionRef('https://github.com/a/b/pull/1'), null);
  assert.equal(parseSessionRef(undefined), null);
});

test('session href parses back to the id', () => {
  assert.deepEqual(parseSessionRef(`#session-${ID}`), { id: ID });
});

test('chip label shortens the 37-char id', () => {
  assert.equal(shortSessionId(ID), `${ID.slice(0, 13)}…`);
  assert.equal(shortSessionId('sess_abc'), 'sess_abc');
});
