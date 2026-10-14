import assert from 'node:assert/strict';
import {test} from 'node:test';
import {bundle} from './helpers/bundle.mjs';
const {applyAcpCompaction,settleAgentCompactions,replayAgentTranscript}=await bundle(`export {applyAcpCompaction,settleAgentCompactions} from './src/lib/acpCompaction'; export {replayAgentTranscript} from './src/transcript/replay';`);
const update=(id,status,patch={})=>({type:'acp_compaction',id,status,summary:null,replace_summary:false,error:null,replace_error:false,...patch});
const chunk=(id,text)=>({type:'acp_compaction_summary',id,text});
test('native updates adopt the admitted placeholder, stream its summary, and replace it on completion',()=>{
 let entries=[{kind:'compact',state:'running',trigger:'agent',summary:null,summarized:null,startedAt:10}];
 entries=applyAcpCompaction(entries,update('s:c','in_progress'),20);
 entries=applyAcpCompaction(entries,chunk('s:c','partial '),30);
 entries=applyAcpCompaction(entries,chunk('s:c','summary'),40);
 assert.equal(entries.length,1);assert.equal(entries[0].summary,'partial summary');assert.equal(entries[0].startedAt,10);
 entries=applyAcpCompaction(entries,update('s:c','completed',{replace_summary:true,summary:'Final retained context'}),50);
 assert.equal(entries[0].state,'done');assert.equal(entries[0].summary,'Final retained context');assert.equal(entries[0].endedAt,50);
 assert.deepEqual(applyAcpCompaction(entries,update('s:c','in_progress'),60),entries);
 assert.deepEqual(applyAcpCompaction(entries,chunk('s:c','late text'),60),entries);
});
test('omission retains fields while explicit replacements clear summary and error',()=>{
 let entries=applyAcpCompaction([],update('s:c','completed',{replace_summary:true,summary:'Summary'}),10);
 entries=applyAcpCompaction(entries,update('s:c','completed'),20);
 assert.equal(entries[0].summary,'Summary');assert.equal(entries[0].endedAt,10);
 entries=applyAcpCompaction(entries,update('s:c','completed',{replace_summary:true,summary:null}),30);
 assert.equal(entries[0].summary,null);
 entries=applyAcpCompaction(entries,update('s:c','failed',{replace_error:true,error:'Failure'}),40);
 assert.equal(entries[0].error,'Failure');
 entries=applyAcpCompaction(entries,update('s:c','failed',{replace_error:true,error:null}),50);
 assert.equal(entries[0].error,null);
});
test('parallel native identities retain their first timeline position',()=>{
 let entries=applyAcpCompaction([],update('one','in_progress'),10);
 entries.push({kind:'msg',msg:{role:'assistant',content:'Reply'}});
 entries=applyAcpCompaction(entries,update('two','in_progress'),20);
 entries=applyAcpCompaction(entries,update('one','failed',{replace_error:true,error:'Provider failure'}),30);
 entries=applyAcpCompaction(entries,update('two','cancelled'),40);
 assert.equal(entries.length,3);assert.equal(entries[0].state,'failed');assert.equal(entries[2].state,'cancelled');
});
test('native receipts, streamed summaries and turn end restore identically on replay',()=>{
 const frames=[update('s:c','in_progress'),chunk('s:c','Remember this'),update('s:c','completed'),{type:'acp_turn_end',stop_reason:'end_turn',detail:null}];
 let live=[];frames.slice(0,3).forEach((f,i)=>live=applyAcpCompaction(live,f,10+i));
 const replay=replayAgentTranscript(frames.map((frame,i)=>({t:10+i,frame})));
 assert.deepEqual(replay.filter(e=>e.kind==='compact'),live);
});
test('a turn end cannot invent success for an unfinished native compaction',()=>{
 const entries=applyAcpCompaction([],update('s:c','in_progress'),10);
 assert.equal(settleAgentCompactions(entries,true,false,'',20)[0].state,'failed');
 assert.equal(settleAgentCompactions(entries,false,true,'Cancelled',20)[0].state,'cancelled');
 const legacy=[{kind:'compact',state:'running',trigger:'agent',summary:null,summarized:null}];
 assert.equal(settleAgentCompactions(legacy,true,false,'',20)[0].state,'done');
});
test('unknown future statuses and stray summary chunks do not pretend completion',()=>{
 assert.deepEqual(applyAcpCompaction([],chunk('missing','ignored')),[]);
 const [entry]=applyAcpCompaction([],update('s:c','_new_status'));
 assert.equal(entry.state,'failed');assert.match(entry.error,/Unsupported compaction status/);
});
