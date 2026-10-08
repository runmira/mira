import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { webcrypto } from 'node:crypto';
import ts from 'typescript';
globalThis.crypto ??= webcrypto;
const { outputText } = ts.transpileModule(readFileSync(new URL('./queueMutations.ts', import.meta.url), 'utf8'), { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } });
const { QueueMutations } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
const input = { type: 'reorder_queued_input', session_id: 'chat-a', id: 'one', before_id: null };
test('offline edits are rejected without sending or replaying', async () => {
  const queue = new QueueMutations();
  let sends = 0;
  await assert.rejects(queue.request(input, () => sends++), /Reconnect/);
  assert.equal(sends, 0);
});
test('only the matching chat acknowledgement resolves a mutation', async () => {
  const queue = new QueueMutations(); queue.setConnected(true);
  let frame, resolved = false;
  const result = queue.request(input, message => { frame = message; }).then(() => { resolved = true; });
  queue.receive({ type: 'queue_mutation_result', session_id: 'chat-b', request_id: frame.request_id, error: null });
  await Promise.resolve(); assert.equal(resolved, false);
  queue.receive({ type: 'queue_mutation_result', session_id: 'chat-a', request_id: frame.request_id, error: null });
  await result; assert.equal(resolved, true);
});
test('server errors and disconnects reject edits without hiding failure', async () => {
  const queue = new QueueMutations(); queue.setConnected(true);
  let frame;
  const failed = queue.request(input, message => { frame = message; });
  queue.receive({ type: 'queue_mutation_result', session_id: 'chat-a', request_id: frame.request_id, error: 'Message already started' });
  await assert.rejects(failed, /already started/);
  const interrupted = queue.request(input, () => {});
  queue.setConnected(false);
  await assert.rejects(interrupted, /Connection lost/);
});
