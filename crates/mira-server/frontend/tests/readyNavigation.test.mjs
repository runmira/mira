import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';

const source = ts.transpileModule(readFileSync(new URL('../src/lib/readyNavigation.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
}).outputText;
const { readyShowsChat } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);

test('switching chats shows the chat; a reload or reconnect keeps the route', () => {
  assert.equal(readyShowsChat('sess_a', 'sess_b'), true, 'picked another chat');
  assert.equal(readyShowsChat('sess_a', 'sess_a'), false, 'reconnect to the same chat');
  assert.equal(readyShowsChat(null, 'sess_a'), false, 'first ready after a page load');
  assert.equal(readyShowsChat('', 'sess_a'), false);
});
