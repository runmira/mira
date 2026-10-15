import assert from 'node:assert/strict';
import {test} from 'node:test';
import {bundle} from './helpers/bundle.mjs';
const {createAppRuntime,openTurnReview,reviewTurnSources}=await bundle(`
export {createAppRuntime} from './src/state/createAppRuntime';
export {openTurnReview} from './src/lib/openTurnReview';
export {reviewTurnSources} from './src/lib/reviewSources';
`);
const scope={backend:'https://mira.test',account:'a'};
const user=text=>({kind:'msg',msg:{role:'user',content:text,created_at:1700000000}});
const summary={text:'PRIVATE_PROMPT',occurrence:0,files:[{path:'a.ts',added:1,removed:1,binary:false}]};
test('panel layout restores by account and workspace without persisting file contents or review comments',async()=>{
 const data=new Map(),storage={getItem:key=>data.get(key),setItem:(key,value)=>data.set(key,value)};
 const first=createAppRuntime(scope,storage);
 first.turnCommands.hydrate('chat',false,[]);first.updateSessionData('cwd','/one');first.updateSessionData('entries',[user(summary.text)]);first.updateSessionData('turnDiffs',[summary]);
 first.updateWorkspaceUi('rightPanelWidth',640);first.updateWorkspaceUi('fileTabs',[{id:'a.ts',path:'a.ts',diff:{before:'PRIVATE_FILE'}}]);
 first.updateWorkspaceUi('toolTabs',[{id:'tool:diff',kind:'diff',title:'Diff'}]);first.updateWorkspaceUi('activeAgentTab','tool:diff');openTurnReview(first,summary);
 first.review.updateDraft('draft',[{text:'PRIVATE_COMMENT'}]);await first.dispose();
 const serialized=[...data.values()].join('');for(const secret of ['PRIVATE_FILE','PRIVATE_PROMPT','PRIVATE_COMMENT']) assert.ok(!serialized.includes(secret));
 const second=createAppRuntime(scope,storage);
 try {
  second.turnCommands.hydrate('chat',false,[]);second.updateSessionData('cwd','/one');
  assert.equal(second.workspaceUi.get().rightPanelWidth,640);assert.equal(second.workspaceUi.get().activeAgentTab,'tool:diff');assert.equal(second.workspaceUi.get().fileTabs[0].diff,null);
  second.updateSessionData('entries',[user(summary.text)]);second.updateSessionData('turnDiffs',[summary]);
  assert.equal(second.workspaceUi.get().reviewTurn.turnKey,'turn:0');
  second.updateSessionData('cwd','/two');assert.equal(second.workspaceUi.get().rightPanelWidth,390);assert.deepEqual(second.workspaceUi.get().fileTabs,[]);
  second.updateSessionData('cwd','/one');assert.equal(second.workspaceUi.get().rightPanelWidth,640);
  const other=createAppRuntime({...scope,account:'b'},storage);other.turnCommands.hydrate('chat',false,[]);other.updateSessionData('cwd','/one');assert.equal(other.workspaceUi.get().rightPanelWidth,390);await other.dispose();
 } finally {await second.dispose();}
});
test('snapshot requests are deduplicated, cached and limited to four across reviews',async()=>{
 const runtime=createAppRuntime(scope);let running=0,peak=0,calls=0;
 globalThis.fetch=async()=>{calls++;running++;peak=Math.max(peak,running);await new Promise(resolve=>setTimeout(resolve,5));running--;return new Response(JSON.stringify({before:'old\n',after:'new\n'}));};
 const turn={...summary,files:Array.from({length:12},(_,i)=>({...summary.files[0],path:`${i}.ts`}))};
 runtime.turnCommands.hydrate('chat',false,[]);runtime.updateSessionData('entries',[user(summary.text),user(summary.text)]);runtime.updateSessionData('turnDiffs',[{...turn,occurrence:1},turn]);
 try {
  const a=runtime.review.loadTurn('chat',turn,new AbortController().signal,'turn:0');const b=runtime.review.loadTurn('chat',turn,new AbortController().signal,'turn:0');
  const result=await Promise.all([a,b]);assert.equal(calls,12);assert.ok(peak<=4);assert.ok(result.flat().every(item=>item.status==='fulfilled'));
  await runtime.review.loadTurn('chat',turn,new AbortController().signal,'turn:0');assert.equal(calls,12);
  await runtime.review.loadTurn('chat',turn,new AbortController().signal,'turn:1');assert.equal(calls,24,'different repeated prompts cannot share a cache entry');
  const aborted=new AbortController();aborted.abort();await runtime.review.loadTurn('chat',turn,aborted.signal,'turn:2');assert.equal(calls,24);
 } finally {await runtime.dispose();}
});
test('turn identities stay stable when another matching prompt is appended',()=>{
 const old={...summary,occurrence:1};
 const before=reviewTurnSources([user(summary.text)],[summary]);
 const after=reviewTurnSources([user(summary.text),user(summary.text)],[summary,old]);
 assert.equal(before[0].key,after[1].key);assert.equal(after[0].key,'turn:1');assert.equal(after[1].timestamp,1700000000);
});
