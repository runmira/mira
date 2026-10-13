import assert from 'node:assert/strict';
import { test } from 'node:test';
import { bundle } from './helpers/bundle.mjs';
const { upsertExternalWork, projectExternalWork, applyExternalChild, projectAcpSubagent, replayExternalStreams, upsertAcpTool, replayAgentTranscript } = await bundle(`
export * from './src/lib/externalSubagents';
export { upsertAcpTool } from './src/transcript/entries';
export { replayAgentTranscript } from './src/transcript/replay';
`);
const work = (id, status = 'running', subagent = null) => ({ id: `subagent:${id}`, native_thread_id: id, kind: 'subagent', status, title: null, subagent });

test('native child identities share Mira agent cards without duplicate rows on snapshots', () => {
  let entries = upsertExternalWork([], work('a', 'running', { name: 'Scout', prompt: 'Inspect files', result: null }));
  entries = upsertExternalWork(entries, work('b'));
  entries = upsertExternalWork(entries, work('a', 'completed', { result: 'Found the bug' }));
  assert.equal(entries.length, 2);
  assert.equal(entries[0].call.function.name, 'agent');
  assert.equal(JSON.parse(entries[0].call.function.arguments).type, 'Scout');
  assert.equal(entries[0].result.content, 'Found the bug');
  assert.equal(entries[1].status, 'running');
});

test('child narration and reused tool IDs stay isolated from siblings', () => {
  let streams = projectExternalWork(new Map(), work('a'));
  streams = projectExternalWork(streams, work('b'));
  streams = applyExternalChild(streams, 'a', { type: 'acp_text', text: 'A only', message_id: 'same' }, 10);
  streams = applyExternalChild(streams, 'b', { type: 'acp_text', text: 'B only', message_id: 'same' }, 11);
  streams = applyExternalChild(streams, 'a', { type: 'acp_thought', text: 'Working', }, 12);
  streams = projectExternalWork(streams, work('a', 'completed'));
  assert.equal(streams.get('external:subagent:a').entries[0].msg.content, 'A only');
  assert.equal(streams.get('external:subagent:b').entries[0].msg.content, 'B only');
  assert.equal(streams.get('external:subagent:a').entries[1].live, false);
  assert.equal(streams.get('external:subagent:b').done, false);
});

test('OpenCode and Claude explicit task calls become Mira delegations with their result', () => {
  for (const name of ['task', 'subagent', 'Task', 'Agent']) {
    const call = { id: name, name, title: 'Investigate', status: 'in_progress', raw_input: { prompt: 'Find failure', subagent_type: 'explore' }, content: [] };
    let entries = upsertAcpTool([], call, 1);
    assert.equal(entries[0].call.function.name, 'agent');
    const done = { ...call, status: 'completed', content: [{ type: 'content', text: 'Fixed it' }] };
    entries = upsertAcpTool(entries, done, 2);
    const streams = projectAcpSubagent(new Map(), done);
    assert.equal(entries.length, 1);
    assert.equal(streams.get(name).done, true);
    assert.equal(streams.get(name).entries[0].msg.content, 'Fixed it');
  }
});

test('ordinary task-management and shell calls remain ordinary tools', () => {
  for (const name of ['Task', 'Bash', 'task_update']) {
    const call = { id: name, name, title: name, status: 'in_progress', raw_input: { command: 'ls' }, content: [] };
    assert.notEqual(upsertAcpTool([], call)[0].call.function.name, 'agent');
    assert.equal(projectAcpSubagent(new Map(), call).size, 0);
  }
});

test('persisted native work and child frames restore cards and panel without parent narration', () => {
  const lines = [
    { t: 1, frame: { type: 'runtime_work_updated', work: work('a') } },
    { t: 2, frame: { type: 'external_subagent_frame', agent_id: 'a', frame: { type: 'acp_text', text: 'Child result', message_id: 'm' } } },
    { t: 3, frame: { type: 'runtime_work_updated', work: work('a', 'completed') } },
  ];
  const entries = replayAgentTranscript(lines);
  assert.equal(entries.filter(e => e.kind === 'tool').length, 1);
  assert.equal(entries.filter(e => e.kind === 'msg').length, 0);
  const restored = replayExternalStreams(lines).get('external:subagent:a');
  assert.equal(restored.entries[0].msg.content, 'Child result');
  assert.equal(restored.done, true);
});


test('completed snapshots stay settled until an explicit child turn reopens them', () => {
  let entries=upsertExternalWork([],work('a','failed'));
  entries=upsertExternalWork(entries,work('a','pending'));
  assert.equal(entries[0].status,'complete');assert.equal(entries[0].result.is_error,true);
  let streams=projectExternalWork(new Map(),work('a','completed'));
  streams=projectExternalWork(streams,work('a','pending'));
  assert.equal(streams.get('external:subagent:a').done,true);
  const reopened=work('a','running',{reopen:true});
  assert.equal(upsertExternalWork(entries,reopened)[0].status,'running');
  assert.equal(projectExternalWork(streams,reopened).get('external:subagent:a').done,false);
});

test('Claude background lifecycle updates reuse the launch call and outlive its tool result', () => {
  const call={id:'launch',name:'Task',title:'Explore',status:'in_progress',raw_input:{prompt:'Inspect code'},content:[]};
  const live={...work('native'),id:'task:native',subagent:{call_id:'launch',reopen:true}};
  let entries=upsertAcpTool([],call);
  entries=upsertExternalWork(entries,live);
  entries=upsertAcpTool(entries,{...call,status:'completed',content:[{type:'content',text:'Started in background'}]});
  assert.equal(entries.length,1);assert.equal(entries[0].status,'running');
  let streams=projectExternalWork(new Map(),live);
  streams=projectAcpSubagent(streams,{...call,status:'completed'});
  assert.equal(streams.get('launch').done,false);
  entries=upsertExternalWork(entries,{...live,status:'completed',subagent:{result:'All done'}});
  assert.equal(entries.length,1);assert.equal(entries[0].result.content,'All done');
  streams=projectExternalWork(streams,{...live,status:'completed',subagent:{result:'All done'}});
  assert.equal(streams.size,1);assert.equal(streams.get('launch').done,true);
  let before=upsertExternalWork([],live);
  before=upsertAcpTool(before,call);
  assert.equal(before.length,1,'lifecycle before tool creation also converges');
});


test('late native call IDs coalesce fallback and launch rows while keeping the open identity', () => {
  const pending=work('late');let entries=upsertExternalWork([],pending);
  const id=entries[0].call.id;
  const call={id:'actual',name:'Task',title:'Task',status:'in_progress',raw_input:{prompt:'Inspect',subagent_type:'explore'},content:[]};
  entries=upsertAcpTool(entries,call);
  let streams=projectExternalWork(new Map(),pending);streams=projectAcpSubagent(streams,call);
  const bound={...pending,subagent:{call_id:'actual'}};
  entries=upsertExternalWork(entries,bound);streams=projectExternalWork(streams,bound);
  assert.equal(entries.length,1);assert.equal(entries[0].call.id,id);assert.equal(JSON.parse(entries[0].call.function.arguments).prompt,'Inspect');
  assert.equal(streams.size,1);assert.ok(streams.has(id));
  entries=upsertAcpTool(entries,{...call,status:'completed'});assert.equal(entries.length,1);assert.equal(entries[0].call.id,id);
});
