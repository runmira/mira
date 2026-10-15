import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const { terminalSignInStatus } = await bundle("export { terminalSignInStatus } from './src/lib/agentAuth';");
const v = (phase, extra = {}) => ({ phase, models: null, ready: null, exit_code: null, ...extra });
test('terminal sign-in reports each outcome without claiming success early', () => {
  assert.equal(terminalSignInStatus(v('running'), true), 'Finish signing in in the terminal below.');
  assert.match(terminalSignInStatus(v('refreshing'), true), /Checking models/);
  assert.equal(terminalSignInStatus(v('succeeded', { ready: true, models: 3 }), true), 'Signed in · 3 models available');
  assert.equal(terminalSignInStatus(v('succeeded', { ready: false, models: 0 }), true), 'Signed in, but the agent is not ready.');
  assert.equal(terminalSignInStatus(v('failed', { exit_code: 1 }), true), 'Sign-in failed (exit 1).');
  assert.equal(terminalSignInStatus(v('cancelled'), true), 'Sign-in cancelled.');
  assert.match(terminalSignInStatus(v('unsupported'), true), /no terminal sign-in/);
  assert.equal(terminalSignInStatus(v('running'), false), 'Enable this agent to sign in.');
});
