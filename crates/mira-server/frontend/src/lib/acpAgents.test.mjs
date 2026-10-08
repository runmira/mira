import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
// The module's one runtime import is a constant the tested code doesn't use.
const source = readFileSync(new URL('./acpAgents.ts', import.meta.url), 'utf8')
  .replace(/^import \{ CLAUDE_MARK \} from '\.\/models';$/m, "const CLAUDE_MARK = '';");
const { outputText } = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } });
const { argsToText, legacyPatch, migrateLegacyAgentConfigs, parseArgs } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);

const KEY = 'mira.acp.instances.v1';
function memoryStorage() {
  const m = new Map();
  return {
    getItem: (k) => (m.has(k) ? m.get(k) : null),
    setItem: (k, v) => m.set(k, String(v)),
    removeItem: (k) => m.delete(k),
  };
}
const empty = (instance) => ({ instance, driver: instance, enabled: true, launch_args: [], env: [], has_api_key: false, api_key_vars: ['X_API_KEY'] });

test('args round-trip through the editable line', () => {
  for (const args of [[], ['-c', 'x=1'], ['--model', 'gpt 5'], ['say "hi"'], ["it's"]]) {
    assert.deepEqual(parseArgs(argsToText(args)), args);
  }
});

test('legacy setup fills only what the server lacks', () => {
  const patch = legacyPatch(
    { apiKey: ' sk-old ', homePath: '~/.codex-work', binaryPath: '/opt/x', launchArgs: '-c x=1', env: 'A=1\nB=2', enabled: false },
    { ...empty('codex'), binary_path: '/usr/bin/codex', env: [{ key: 'A', value: 'server' }] },
  );
  assert.equal(patch.api_key, 'sk-old');
  assert.equal(patch.home_path, '~/.codex-work');
  assert.equal(patch.binary_path, undefined, 'the server value wins');
  assert.deepEqual(patch.launch_args, ['-c', 'x=1']);
  assert.deepEqual(patch.env, { A: null, B: '2' }, 'stored A kept, B added');
  assert.equal(patch.enabled, false);
  assert.equal(legacyPatch({ apiKey: 'sk-old' }, { ...empty('codex'), has_api_key: true }).api_key, undefined);
});

test('migration moves setups to the server and clears the browser copy', async () => {
  globalThis.localStorage = memoryStorage();
  localStorage.setItem(KEY, JSON.stringify({ codex: { apiKey: 'sk-1', enabled: true }, 'claude-code': { apiKey: 'sk-2' } }));
  const puts = [];
  const moved = await migrateLegacyAgentConfigs(
    async (instance, patch) => { puts.push([instance, patch]); },
    async (instance) => empty(instance),
  );
  assert.deepEqual(moved.sort(), ['claude-code', 'codex']);
  assert.deepEqual(puts.map(([i, p]) => [i, p.api_key]).sort(), [['claude-code', 'sk-2'], ['codex', 'sk-1']]);
  assert.equal(localStorage.getItem(KEY), null);
});

test('a failed save keeps that agent for the next try; unknown agents are dropped', async () => {
  globalThis.localStorage = memoryStorage();
  localStorage.setItem(KEY, JSON.stringify({ codex: { apiKey: 'sk-1' }, gone: { apiKey: 'sk-9' } }));
  const moved = await migrateLegacyAgentConfigs(
    async () => { throw new Error('agent settings 500'); },
    async (instance) => {
      if (instance === 'gone') throw new Error('unknown engine instance `gone`');
      return empty(instance);
    },
  );
  assert.deepEqual(moved, []);
  assert.deepEqual(Object.keys(JSON.parse(localStorage.getItem(KEY))), ['codex']);
});
