import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';

const source = readFileSync(new URL('../src/lib/settled.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ESNext },
});
const { isSettled } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);

test('resumed locally merged chat stays live after a turn without commits', () => {
  const chat = { worktree_status: 'merged', updated_at: 2000 };
  assert.equal(isSettled(chat), true);
  assert.equal(isSettled({ ...chat, unsettled_at: 1000 }), false);
});

test('user input immediately after closing keeps the chat live after completion', () => {
  const chat = { pr: { state: 'merged', closed_at: 1000 }, updated_at: 1100 };
  assert.equal(isSettled(chat), true);
  assert.equal(isSettled({ ...chat, unsettled_at: 1001 }), false);
  assert.equal(isSettled({ ...chat, pr: { state: 'merged', closed_at: 1200 }, unsettled_at: 1001 }), true);
});

test('resuming a manually settled chat clears its mark', () => {
  assert.equal(isSettled({ settled_at: 1000, updated_at: 1000 }), true);
  assert.equal(isSettled({ unsettled_at: 1001, updated_at: 1100 }), false);
});
