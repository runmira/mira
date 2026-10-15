import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const { describeAgentStatus, versionLabel } = await bundle(`export { describeAgentStatus, versionLabel } from './src/lib/acpAgents';`);
const ready = { kind: 'opencode', display_name: 'OpenCode', state: { state: 'ready' }, auth_method_ids: [], launch: 'opencode acp', version: '2.0.22' };

test('a started agent with an unsigned provider warns instead of reading ready', () => {
  assert.deepEqual(describeAgentStatus(ready), { text: 'v2.0.22', tone: 'ok' });
  const needs = [{ provider: 'anthropic', model: 'anthropic/claude-x', source: 'turn', reason: 'Authentication required' }];
  assert.deepEqual(describeAgentStatus({ ...ready, sign_in_needed: needs }), { text: 'Sign-in needed · anthropic/claude-x', tone: 'warn' });
  const two = [...needs, { provider: 'opencode', model: 'opencode/cat-jam-free', source: 'turn', reason: 'x' }];
  assert.deepEqual(describeAgentStatus({ ...ready, sign_in_needed: two }), { text: 'Sign-in needed · 2 models', tone: 'warn' });
  const unknown = [{ provider: null, source: 'turn', reason: 'Not logged in' }];
  assert.deepEqual(describeAgentStatus({ ...ready, sign_in_needed: unknown }), { text: 'Sign-in needed', tone: 'warn' });
});

test('agent versions read as a version, not a user agent', () => {
  assert.equal(versionLabel('mira-probe/0.162.0-alpha.17.2 (Mac OS 26.3.0; arm64) Apple_Terminal/455.1 (mira-probe; 0.6.2)'), 'v0.162.0-alpha.17.2');
  assert.equal(versionLabel('2.0.22'), 'v2.0.22');
  assert.equal(versionLabel('nightly'), 'nightly');
  assert.equal(versionLabel(null), null);
});
