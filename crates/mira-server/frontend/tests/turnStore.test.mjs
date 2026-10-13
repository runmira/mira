import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const { createTurnStore, selectTurn } = await bundle(`export { createTurnStore, selectTurn } from './src/state/turnStore';`);

test('hydration and start/stop commit coherent snapshots without mirrored busy state', () => {
  const turns = createTurnStore();
  const snapshots = [];
  const sub = turns.source.subscribe(state => snapshots.push(selectTurn(state)));
  turns.commands.hydrate('a', true, []);
  assert.equal(snapshots.length, 1);
  assert.equal(snapshots[0].thinking, true);
  assert.equal(turns.busy.current, true);
  const generation = turns.source.get().generation;
  turns.commands.setBusy(false);
  assert.deepEqual([selectTurn(turns.source.get()).busy, selectTurn(turns.source.get()).thinking], [false, false]);
  turns.commands.setThinking(true);
  assert.equal(snapshots.length, 2);
  turns.commands.setBusy(true);
  assert.equal(snapshots.length, 3);
  assert.ok(turns.source.get().generation > generation);
  assert.equal(selectTurn(turns.source.get()).thinking, true);
  sub.unsubscribe();
});

test('session switches retain running projections and Ready authoritatively reconciles them', () => {
  const turns = createTurnStore();
  turns.commands.hydrate('a', true, []);
  const a = turns.source.get().sessions.get('a');
  const generation = turns.source.get().generation;
  turns.commands.suspend();
  turns.commands.setThinking(true);
  assert.equal(turns.busy.current, false);
  assert.equal(turns.source.get().sessions.get('a'), a);
  turns.commands.hydrate('b', false, []);
  assert.equal(turns.source.get().sessions.get('a'), a);
  assert.ok(turns.source.get().generation > generation);
  turns.commands.hydrate('a', false, []);
  assert.equal(turns.busy.current, false);
});

test('native work and overlapping native turns preserve remaining work and late completion does not undo Stop', () => {
  const turns = createTurnStore();
  turns.commands.hydrate('a', false, []);
  turns.commands.nativeTurn('one', true);
  turns.commands.nativeTurn('two', true);
  const old = selectTurn(turns.source.get());
  turns.commands.nativeTurn('one', false);
  assert.equal(old.nativeTurns.has('one'), true);
  turns.commands.nativeWork({ id: 'work', status: 'running' });
  turns.commands.finishAgent();
  assert.equal(turns.busy.current, true);
  assert.equal(selectTurn(turns.source.get()).thinking, false);
  turns.commands.setBusy(false);
  turns.commands.nativeWork({ id: 'work', status: 'completed' });
  assert.equal(turns.busy.current, false);
  turns.commands.nativeTurn('two', false);
  turns.commands.finishAgent();
  assert.equal(turns.busy.current, false);
  turns.commands.nativeWork({ id: 'work', status: 'completed' });
  assert.equal(turns.busy.current, false);
});

test('idle retention is bounded while running sessions survive; disposal blocks updates', () => {
  const turns = createTurnStore();
  turns.commands.hydrate('running', true, []);
  for (let i = 0; i < 100; i++) turns.commands.hydrate(String(i), false, []);
  assert.equal(turns.source.get().sessions.size, 21);
  assert.equal(turns.source.get().sessions.get('running').busy, true);
  const snapshot = turns.source.get();
  turns.dispose();
  turns.commands.hydrate('new', true, []);
  turns.commands.nativeTurn('new', true);
  turns.commands.setBusy(true);
  assert.equal(turns.source.get(), snapshot);
});
