import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const { describeAgentStatus } = await bundle(`export { describeAgentStatus } from './src/lib/acpAgents';`);
const ready = { kind: 'opencode', display_name: 'OpenCode', state: { state: 'ready' }, auth_method_ids: [], launch: 'opencode acp', version: '2.0.22' };

test('a started agent with an unsigned provider warns instead of reading ready', () => {
  assert.deepEqual(describeAgentStatus(ready), { text: 'v2.0.22', tone: 'ok' });
  const needs = [{ provider: 'anthropic', model: 'anthropic/claude-x', source: 'turn', reason: 'Authentication required' }];
  assert.deepEqual(describeAgentStatus({ ...ready, sign_in_needed: needs }), { text: 'Sign-in needed · anthropic', tone: 'warn' });
  const unknown = [{ provider: null, source: 'turn', reason: 'Not logged in' }];
  assert.deepEqual(describeAgentStatus({ ...ready, sign_in_needed: unknown }), { text: 'Sign-in needed', tone: 'warn' });
});
