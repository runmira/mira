import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const { enabledAgentRail, externalAgentStatuses } = await bundle(`export { enabledAgentRail, externalAgentStatuses } from './src/lib/enginePickerRail';`);
const row = (instance, driver = instance, enabled = true) => ({ instance, driver, enabled, display_name: instance, flavor: 'external', models: [] });
test('enabled agents appear before probes finish; disabled agents stay hidden', () => {
  const rail = enabledAgentRail([row('codex'), row('claude-code'), row('opencode'), row('cursor', 'cursor', false)], [], null);
  assert.deepEqual(rail.map(r => r.driver), ['codex', 'claude-code', 'opencode']);
});
test('separate enabled instances retain their settings identity instead of collapsing by driver', () => {
  const rail = enabledAgentRail([row('personal', 'codex'), row('work', 'codex')], [], {kind:'agent', driver:'codex', instance:'work', display_name:'Work'});
  assert.deepEqual(rail.map(r => [r.key, r.instance]), [['agent:personal','personal'], ['agent:work','work']]);
});

test('settings retain unprobed and timed-out agents instead of dropping them into an endless spinner', () => {
  const pending = {...row('codex'), state:{state:'failed',reason:'checking…'}};
  const timeout = {...row('claude-code'), state:{state:'failed',reason:'probe did not finish within 75s'}};
  assert.deepEqual(externalAgentStatuses([pending,timeout]).map(s=>[s.kind,s.state.reason]), [['codex','checking…'],['claude-code','probe did not finish within 75s']]);
});
