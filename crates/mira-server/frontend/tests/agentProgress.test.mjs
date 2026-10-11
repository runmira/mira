import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';

const source = ts.transpileModule(readFileSync(new URL('../src/lib/agentProgress.ts', import.meta.url), 'utf8'), { compilerOptions: { module: ts.ModuleKind.ESNext } }).outputText;
const { appendAgentProgress, agentFailureEntry, hasCurrentAgentFailure } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
const progress = (kind) => ({ type: 'stream_activity', kind, title: kind, detail: '' });

test('prompt acknowledgements replace progress without moving backwards', () => {
  let entries = [];
  for (const stage of ['prompt_preparing', 'prompt_queued', 'prompt_started', 'prompt_sent']) entries = appendAgentProgress(entries, progress(stage));
  assert.equal(entries.length, 1);
  assert.equal(entries[0].activityKind, 'prompt_started');
});

test('the next message and provider retries retain their own progress', () => {
  const entries = appendAgentProgress([], progress('prompt_completed'));
  const next = appendAgentProgress(entries, progress('prompt_preparing'));
  assert.equal(next.length, 2);
  assert.equal(appendAgentProgress(next, progress('api_retry')).length, 3);
});

test('specific failures retain reset and retry metadata and prevent generic duplicates only for this turn', () => {
  const failure = agentFailureEntry({ type: 'acp_failure', code: 'api_error_429', message: 'Limit reached', retryable: true, reset_at: '2026-10-11T22:00:00Z' });
  assert.equal(failure.retryable, true);
  assert.equal(failure.resetAt, '2026-10-11T22:00:00Z');
  assert.equal(hasCurrentAgentFailure([failure]), true);
  assert.equal(hasCurrentAgentFailure([failure, { kind: 'msg', msg: { role: 'user', content: 'New request' } }]), false);
  assert.equal(hasCurrentAgentFailure([{ kind: 'error', text: 'Settings failed' }]), false);
});
