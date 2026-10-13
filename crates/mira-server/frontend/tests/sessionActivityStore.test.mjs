import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const { createSessionActivityStore, createAppRuntime } = await bundle(`
export { createSessionActivityStore } from './src/state/sessionActivityStore';
export { createAppRuntime } from './src/state/createAppRuntime';
`);
const snapshot = (revision, running, epoch = 'one') => ({ snapshot: { epoch, revision, running } });
const delta = (revision, session_id, running, epoch = 'one') => ({ epoch, revision, session_id, running });

test('activity and completion transition atomically; stale/replayed frames do not re-notify', () => {
  const activity = createSessionActivityStore();
  activity.apply(snapshot(1, ['a', 'b']));
  const previous = activity.source.get();
  let calls = 0;
  const subscription = activity.source.subscribe(state => {
    calls++;
    assert.equal(state.activity.running.has('a'), false);
    assert.equal(state.completed.get('a'), 2);
    assert.equal(state.activity.running.has('b'), true);
  });
  assert.deepEqual(activity.apply(delta(2, 'a', false)), ['a']);
  assert.equal(calls, 1);
  assert.equal(previous.activity.running.has('a'), true);
  assert.equal(previous.completed.size, 0);
  const settled = activity.source.get();
  for (const update of [delta(2, 'a', false), delta(1, 'a', true), snapshot(1, ['a', 'b']), snapshot(2, ['b'])]) {
    assert.deepEqual(activity.apply(update), []);
    assert.equal(activity.source.get(), settled);
  }
  subscription.unsubscribe();
});

test('reconnect snapshot reconciles multiple sessions and epoch restart clears completion revisions', () => {
  const activity = createSessionActivityStore();
  assert.deepEqual(activity.apply(delta(1, 'unknown', true)), []);
  assert.equal(activity.source.get().activity, null);
  activity.apply(snapshot(5, ['a', 'b', 'c']));
  assert.deepEqual(activity.apply(snapshot(8, ['c'])), ['a', 'b']);
  assert.equal(activity.source.get().completed.get('b'), 8);
  const state = activity.source.get();
  activity.apply(delta(100, 'a', true, 'old'));
  assert.equal(activity.source.get(), state);
  assert.deepEqual(activity.apply(snapshot(1, ['a'], 'two')), []);
  assert.equal(activity.source.get().completed.size, 0);
  assert.deepEqual([...activity.source.get().activity.running], ['a']);
});

test('completion retention is bounded and revision-only updates preserve running selection', () => {
  const activity = createSessionActivityStore();
  const ids = Array.from({ length: 600 }, (_, i) => String(i));
  activity.apply(snapshot(1, ids));
  const running = activity.source.get().activity.running;
  activity.apply(snapshot(2, ids));
  assert.equal(activity.source.get().activity.running, running);
  assert.equal(activity.apply(snapshot(3, [])).length, 600);
  assert.equal(activity.source.get().completed.size, 500);
  assert.equal(activity.source.get().completed.has('0'), false);
  assert.equal(activity.source.get().completed.get('599'), 3);
});

test('activity is account isolated and disposed commands cannot mutate or emit completions', async () => {
  const a = createAppRuntime({ backend: 'one', account: 'a' });
  const b = createAppRuntime({ backend: 'one', account: 'b' });
  a.commands.applySessionActivity(snapshot(1, ['same']));
  assert.equal(b.sessionActivity.get().activity, null);
  const state = a.sessionActivity.get();
  await a.dispose();
  assert.deepEqual(a.commands.applySessionActivity(delta(2, 'same', false)), []);
  assert.equal(a.sessionActivity.get(), state);
  await b.dispose();
});
