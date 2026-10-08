import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = readFileSync(new URL('./turnActivity.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext } });
const { turnActivity, groupByTurn } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
const text = { kind: 'msg', nativePhase: 'commentary', msg: { role: 'assistant', content: 'Checking the project.' } };
const tool = (id, status = 'running', native = true) => ({ kind: 'tool', call: { id, function: { name: 'bash', arguments: '{}' } }, preview: null, result: null, status, ...(native ? { agentCall: { id } } : {}) });
test('tool parent is present from the first native start after assistant text, before completion', () => {
  const live = turnActivity([text, tool('a')], false);
  assert.equal(live.finalEntry, text);
  assert.equal(live.trailing[0].kind, 'tool-group');
  assert.equal(live.trailing[0].entries[0].status, 'running');
});
test('parallel starts and updates stay grouped while the turn is streaming', () => {
  const starts = [tool('a'), tool('b')];
  const live = turnActivity([text, ...starts], false);
  assert.equal(live.trailing.length, 1);
  assert.equal(live.trailing[0].kind, 'tool-group');
  assert.deepEqual(live.trailing[0].entries, starts);
  const updated = turnActivity([text, { ...starts[0], activityAt: 3 }, starts[1]], false);
  assert.equal(updated.trailing[0].entries[0].activityAt, 3);
  const finished = turnActivity([text, tool('a', 'complete'), tool('b', 'complete')], true);
  assert.equal(finished.intermediate[0].kind, 'tool-group');
  assert.equal(finished.trailing.length, 0);
});
test('provider tools before and after assistant text use the same grouping', () => {
  const tools = [tool('a', 'running', false), tool('b', 'running', false)];
  assert.equal(turnActivity([...tools, text], false).intermediate[0].kind, 'tool-group');
  assert.equal(turnActivity([text, ...tools], false).trailing[0].kind, 'tool-group');
});
test('approval and question cards remain visible outside a closed tool parent', () => {
  const pending = tool('approve', 'pending');
  const question = { ...tool('question'), askUser: { decision: null } };
  const live = turnActivity([text, tool('a'), pending, question, tool('b')], false);
  assert.deepEqual(live.trailing.map(item => item.kind), ['tool-group', 'entry', 'entry', 'tool-group']);
  assert.equal(live.trailing[1].entry, pending);
  assert.equal(live.trailing[2].entry, question);
});

test('steering enters the current response in chronological order without creating a turn header', () => {
  const user = { kind: 'msg', msg: { role: 'user', content: 'Original' } };
  const steer = { kind: 'msg', msg: { role: 'user', content: 'Use another layout', input_intent: 'steer' } };
  const next = { kind: 'msg', msg: { role: 'user', content: 'Next ordinary turn' } };
  const turns = groupByTurn([user, text, tool('a'), steer, tool('b'), next]);
  assert.equal(turns.length, 2);
  assert.equal(turns[0].user, user);
  assert.deepEqual(turns[0].body, [text, tool('a'), steer, tool('b')]);
  assert.equal(turns[1].user, next);
});
