import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';

const source = ts.transpileModule(readFileSync(new URL('../src/lib/authReturn.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
}).outputText;
const { returnUrl } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);

test('an OAuth return gets back the route it left from', () => {
  assert.equal(returnUrl('http://localhost:5173/?code=abc', '#/plugins'), '/?code=abc#/plugins');
  assert.equal(returnUrl('http://localhost:5173/?code=abc#/', '#/work/%2Frepo%23208'), '/?code=abc#/work/%2Frepo%23208');
  assert.equal(returnUrl('http://localhost:5173/?error=access_denied', '#/settings/integrations'), '/?error=access_denied#/settings/integrations');
});

test('anything else is left alone', () => {
  assert.equal(returnUrl('http://localhost:5173/#/chat', '#/plugins'), null, 'not an OAuth return');
  assert.equal(returnUrl('http://localhost:5173/?code=abc#/chat', '#/plugins'), null, 'already has a route');
  assert.equal(returnUrl('http://localhost:5173/?code=abc', null), null, 'nothing remembered');
  assert.equal(returnUrl('http://localhost:5173/?code=abc', '#/'), null, 'nothing worth restoring');
  assert.equal(returnUrl('http://localhost:5173/?code=abc', 'javascript:alert(1)'), null);
});
