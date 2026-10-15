import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const store = new Map();
globalThis.localStorage = { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, String(v)), removeItem: (k) => store.delete(k) };
const { recentModels, rememberModel, pickerAgentSubtitle } = await bundle(`
export { recentModels, rememberModel } from './src/lib/pickerRecents';
export { pickerAgentSubtitle } from './src/lib/acpAgents';`);

test('recent picks are per engine, newest first, deduplicated and capped', () => {
  for (const m of ['a', 'b', 'c', 'a', 'd', 'e', 'f']) rememberModel('provider:openrouter', m);
  assert.deepEqual(recentModels('provider:openrouter'), ['f', 'e', 'd', 'a', 'c']);
  assert.deepEqual(recentModels('agent:codex'), []);
  store.set('mira.picker.recent.v1', '{not json');
  assert.deepEqual(recentModels('provider:openrouter'), [], 'corrupt storage reads as empty');
});

test('the picker names who an agent runs as, not its version or transport', () => {
  const ready = { kind: 'codex', display_name: 'Codex', state: { state: 'ready' }, auth_method_ids: [], launch: '', version: 'mira/0.162.0 (Mac OS)', transport: 'native' };
  assert.equal(pickerAgentSubtitle({ ...ready, auth: 'you@example.com · plus' }), 'you@example.com · plus');
  assert.equal(pickerAgentSubtitle(ready), 'Uses its own sign-in');
  const needs = [{ provider: 'opencode', model: 'opencode/cat-jam-free', source: 'turn', reason: 'x' }];
  assert.equal(pickerAgentSubtitle({ ...ready, sign_in_needed: needs }), 'Sign-in needed · opencode/cat-jam-free', 'problems still show');
});
