import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';
const source = ts.transpileModule(readFileSync(new URL('../src/lib/chatFailure.ts', import.meta.url), 'utf8'), { compilerOptions: { module: ts.ModuleKind.ESNext } }).outputText;
const { chatFailure, isFatalChatWarning, isProviderLimitFailure } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);

test('ordinary warnings remain transcript notes; native failures get recovery', () => {
  for (const text of ['provider error: rejected', 'stream error: reset', 'stream timed out after 300s']) assert.equal(isFatalChatWarning(text), true);
  for (const text of ['[hook] blocked', '[verify] running', '[progress] read a file']) assert.equal(isFatalChatWarning(text), false);
});
test('auth failures direct users to connection settings, transient errors explain retry', () => {
  assert.equal(chatFailure('provider error: 401 invalid API key').settings, true);
  assert.equal(chatFailure('stream timed out after 300s').title, 'The response timed out');
  assert.equal(chatFailure('stream error: connection reset').title, 'The response was interrupted');
  assert.equal(chatFailure('agent turn failed: malformed tool call').title, 'The request could not finish');
});
test('existing provider-limit recovery only replaces limit notices', () => {
  for (const text of ['provider error: 429', 'usage limit reached', 'rate_limit exceeded', 'quota exceeded']) assert.equal(isProviderLimitFailure(text), true);
  assert.equal(isProviderLimitFailure('stream error: connection reset'), false);
});
