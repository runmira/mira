import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const { enabledAgentRail, enabledPickerRail, externalAgentStatuses } = await bundle(`export { enabledAgentRail, enabledPickerRail, externalAgentStatuses } from './src/lib/enginePickerRail';`);
const row = (instance, driver = instance, enabled = true) => ({ instance, driver, enabled, display_name: instance, flavor: 'external', models: [] });
test('enabled agents appear before probes finish; disabled agents stay hidden', () => {
  const rail = enabledAgentRail([row('codex'), row('claude-code'), row('opencode'), row('cursor', 'cursor', false)], [], null);
  assert.deepEqual(rail.map(r => r.driver), ['claude-code', 'codex', 'opencode']);
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

test('only enabled registry instances appear; active and discovered disabled agents are never reinserted', () => {
  const engines=[row('codex','codex',false),row('claude-code'),{...row('hidden'),flavor:'native',enabled:false}];
  assert.deepEqual(enabledPickerRail(engines,[{kind:'codex',display_name:'Codex'}]).map(i=>i.instance),['claude-code']);
  assert.deepEqual(enabledAgentRail(null,[{kind:'codex',display_name:'Codex'}]),[]);
  assert.deepEqual(enabledPickerRail([],[]),[]);
});
test('alphabetical provider and agent order is independent of registry and health response order', () => {
  const engines=[
    {...row('z-provider'),flavor:'native',display_name:'Z Provider'},
    {...row('a-provider'),flavor:'native',display_name:'A Provider'},
    {...row('work','codex'),display_name:'Codex'},
    {...row('personal','codex'),display_name:'Codex'},
    {...row('claude-code'),display_name:'Claude Code'},
  ];
  const expected=['a-provider','z-provider','claude-code','personal','work'];
  assert.deepEqual(enabledPickerRail(engines,[]).map(i=>i.instance),expected);
  assert.deepEqual(enabledPickerRail([...engines].reverse(),[{kind:'codex',display_name:'Codex'}]).map(i=>i.instance),expected);
});
