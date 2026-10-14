import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const { forkActions } = await bundle("export { forkActions } from './src/lib/agentFork';");
const agent = fork => ({ kind: 'agent', driver: 'x', display_name: 'X', status: 'ready', capabilities: fork === undefined ? undefined : { fork } });
test('providers fork from any message and have no whole-chat agent fork', () => {
  assert.deepEqual(forkActions({ kind: 'provider', display_name: 'Mira', status: 'ready' }), { fromMessage: true, wholeChat: false });
});
test('agents offer exactly the fork scope their runtime negotiated', () => {
  assert.deepEqual(forkActions(agent('turn')), { fromMessage: true, wholeChat: true });
  assert.deepEqual(forkActions(agent('session')), { fromMessage: false, wholeChat: true });
  assert.deepEqual(forkActions(agent('unavailable')), { fromMessage: false, wholeChat: false });
  // Unknown (older server, not yet started): conservative.
  assert.deepEqual(forkActions(agent(undefined)), { fromMessage: false, wholeChat: false });
});
