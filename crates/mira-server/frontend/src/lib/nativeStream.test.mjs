import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = readFileSync(new URL('./nativeStream.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext } });
const { applyNativeMetadata, appendNativeText, appendNativeToolOutput, boundedOutput, MAX_TOOL_OUTPUT } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
const user = { kind: 'msg', msg: { role: 'user', content: 'Build it' } };
test('adjacent commentary and final message retain native boundaries', () => {
  const entries = appendNativeText(appendNativeText([user], 'Looking around.', 'commentary'), 'Done.', 'final');
  assert.equal(entries.length, 3);
  assert.equal(entries[1].msg.content, 'Looking around.');
  assert.equal(entries[2].msg.content, 'Done.');
});
test('interleaved work does not split a native message or move work', () => {
  const work = { kind: 'thought', text: 'Checking', live: false };
  const entries = appendNativeText([...appendNativeText([user], 'First ', 'item'), work], 'second', 'item');
  assert.equal(entries.length, 3);
  assert.equal(entries[1].msg.content, 'First second');
  assert.equal(entries[2], work);
});
test('reused IDs never patch an earlier user turn', () => {
  const entries = appendNativeText([...appendNativeText([user], 'Old', 'item'), user], 'New', 'item');
  assert.equal(entries[1].msg.content, 'Old');
  assert.equal(entries.at(-1).msg.content, 'New');
});
test('empty deltas do not create blank messages', () => {
  const entries = [user];
  assert.equal(appendNativeText(entries, '', 'item'), entries);
});

test('completed snapshots correct missing chunks without duplicating text', () => {
  const partial = appendNativeText([user], 'Partial', 'item');
  const complete = appendNativeText(partial, 'Partial answer completed', 'item', true);
  assert.equal(complete.length, 2);
  assert.equal(complete[1].msg.content, 'Partial answer completed');
  assert.equal(appendNativeText(complete, 'Partial answer completed', 'item', true), complete);
  assert.equal(appendNativeText(complete, 'late chunk', 'item'), complete);
});
test('snapshot without prior deltas restores the entire message', () => {
  const entries = appendNativeText([user], 'Recovered answer', 'item', true);
  assert.equal(entries[1].msg.content, 'Recovered answer');
});
test('live output accumulates and keeps the newest bounded tail', () => {
  const tool = {kind:'tool',call:{id:'cmd'},status:'running',result:null};
  const entries = appendNativeToolOutput(appendNativeToolOutput([tool], 'cmd', 'one'), 'cmd', 'two');
  assert.equal(entries[0].result.content, 'onetwo');
  const tail = boundedOutput('x'.repeat(MAX_TOOL_OUTPUT * 2), 'newest');
  assert.ok(tail.startsWith('[Earlier output truncated]'));
  assert.ok(tail.endsWith('newest'));
  assert.ok(tail.length < MAX_TOOL_OUTPUT + 40);
  assert.equal(appendNativeToolOutput([{...tool,status:'complete',result:{content:'final'}}], 'cmd', 'late')[0].result.content, 'final');
});

test('native phase metadata is scoped to the current turn when IDs repeat', () => {
  const first = appendNativeText([user], 'Earlier answer', 'shared', true);
  const second = appendNativeText([...first, user], 'Current answer', 'shared', true);
  const marked = applyNativeMetadata(second, 'shared', 'final_answer');
  assert.equal(marked[1].nativePhase, undefined);
  assert.equal(marked[3].nativePhase, 'final_answer');
  assert.equal(marked[1], first[1]);
});

test('message time remains stable across deltas and authoritative snapshots', () => {
  const first = appendNativeText([user], 'First', 'timed');
  assert.ok(first[1].msg.created_at > 0);
  const time = first[1].msg.created_at;
  const next = appendNativeText(first, ' chunk', 'timed');
  assert.equal(next[1].msg.created_at, time);
  assert.equal(appendNativeText(next, 'Complete', 'timed', true)[1].msg.created_at, time);
});
