import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const { createRuntimeStore, selectActiveSession } = await bundle(`export { createRuntimeStore, selectActiveSession } from './src/state/runtimeStore';`);

test('hydration and start/stop commit coherent snapshots without mirrored busy state', () => {
  const turns = createRuntimeStore();
  const snapshots = [];
  const sub = turns.source.subscribe(state => snapshots.push(selectActiveSession(state)));
  turns.commands.hydrate('a', true, []);
  assert.equal(snapshots.length, 1);
  assert.equal(snapshots[0].thinking, true);
  assert.equal(turns.busy.current, true);
  const generation = turns.source.get().generation;
  turns.commands.reportTurnRunning(false);
  assert.deepEqual([selectActiveSession(turns.source.get()).busy, selectActiveSession(turns.source.get()).thinking], [false, false]);
  turns.commands.reportThinking(true);
  assert.equal(snapshots.length, 2);
  turns.commands.reportTurnRunning(true);
  assert.equal(snapshots.length, 3);
  assert.ok(turns.source.get().generation > generation);
  assert.equal(selectActiveSession(turns.source.get()).thinking, true);
  sub.unsubscribe();
});

test('session switches retain running projections and Ready authoritatively reconciles them', () => {
  const turns = createRuntimeStore();
  turns.commands.hydrate('a', true, []);
  const a = turns.source.get().sessions.get('a');
  const generation = turns.source.get().generation;
  turns.commands.suspend();
  turns.commands.reportThinking(true);
  assert.equal(turns.busy.current, false);
  assert.equal(turns.source.get().sessions.get('a'), a);
  turns.commands.hydrate('b', false, []);
  assert.equal(turns.source.get().sessions.get('a'), a);
  assert.ok(turns.source.get().generation > generation);
  turns.commands.hydrate('a', false, []);
  assert.equal(turns.busy.current, false);
});

test('native work and overlapping native turns preserve remaining work and late completion does not undo Stop', () => {
  const turns = createRuntimeStore();
  turns.commands.hydrate('a', false, []);
  turns.commands.nativeTurn('one', true);
  turns.commands.nativeTurn('two', true);
  const old = selectActiveSession(turns.source.get());
  turns.commands.nativeTurn('one', false);
  assert.equal(old.nativeTurns.has('one'), true);
  turns.commands.nativeWork({ id: 'work', status: 'running' });
  turns.commands.finishAgent();
  assert.equal(turns.busy.current, true);
  assert.equal(selectActiveSession(turns.source.get()).thinking, false);
  turns.commands.reportTurnRunning(false);
  turns.commands.nativeWork({ id: 'work', status: 'completed' });
  assert.equal(turns.busy.current, false);
  turns.commands.nativeTurn('two', false);
  turns.commands.finishAgent();
  assert.equal(turns.busy.current, false);
  turns.commands.nativeWork({ id: 'work', status: 'completed' });
  assert.equal(turns.busy.current, false);
});

test('idle retention is bounded while running sessions survive; disposal blocks updates', () => {
  const turns = createRuntimeStore();
  turns.commands.hydrate('running', true, []);
  for (let i = 0; i < 100; i++) turns.commands.hydrate(String(i), false, []);
  assert.equal(turns.source.get().sessions.size, 21);
  assert.equal(turns.source.get().sessions.get('running').busy, true);
  turns.dispose();
  const snapshot = turns.source.get();
  turns.commands.hydrate('new', true, []);
  turns.commands.nativeTurn('new', true);
  turns.commands.reportTurnRunning(true);
  assert.equal(turns.source.get(), snapshot);
});
