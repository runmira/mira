import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';
const output = ts.transpileModule(await readFile(new URL('./toolActivity.ts', import.meta.url), 'utf8'), { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
const { plainToolAction, latestToolEntry } = await import(`data:text/javascript;base64,${Buffer.from(output).toString('base64')}`);
const call = (name, args = {}) => ({ id: name, function: { name, arguments: JSON.stringify(args) } });
test('parallel parent follows event ordering rather than the last tool in the array', () => {
  const entries = [{ call: call('read_file'), activityAt: 4 }, { call: call('bash'), activityAt: 2 }];
  assert.equal(latestToolEntry(entries), entries[0]);
  assert.equal(latestToolEntry([]), undefined);
});
test('activity labels explain commands and handle incomplete streamed arguments', () => {
  assert.equal(plainToolAction(call('bash', { command: 'cargo test -p mira-server' }), true), 'Running tests');
  assert.equal(plainToolAction(call('bash', { command: 'npm run build' }), false), 'Checked the project');
  assert.equal(plainToolAction(call('read_file', { path: 'App.tsx' }), true), 'Reading App.tsx');
  assert.equal(plainToolAction({ ...call('bash'), function: { name: 'bash', arguments: '{' } }, true), 'Running a command');
});
const { completedToolSummary, toolActivityKind } = await import(`data:text/javascript;base64,${Buffer.from(output).toString('base64')}`);
test('completed groups describe every action category once in past tense', () => {
  const entries = ['Read','read_file','mcp__browser__web_search','exec_command'].map(name => ({call:call(name),status:'complete'}));
  assert.equal(completedToolSummary(entries),'Read files, searched the web, ran a command');
  assert.equal(toolActivityKind('unknown_extension_tool'),'other');
  assert.equal(toolActivityKind('functions.apply_patch'),'edit');
  assert.equal(toolActivityKind('context_compaction'),'compact');
});
test('failed and denied calls do not become successful activity claims', () => {
  assert.equal(completedToolSummary([{call:call('bash'),status:'complete',result:{is_error:true}}]),'Tools did not complete');
  assert.equal(completedToolSummary([{call:call('read_file'),status:'complete'},{call:call('bash'),status:'denied'}]),'Read files; some tools did not complete');
});
