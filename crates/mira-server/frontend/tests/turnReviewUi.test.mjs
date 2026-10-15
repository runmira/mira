import { QueryClientProvider } from '@tanstack/react-query';
import assert from 'node:assert/strict';
import {test} from 'node:test';
import React,{act} from 'react';
import {createRoot} from 'react-dom/client';
import {Window} from 'happy-dom';
import {bundle} from './helpers/bundle.mjs';
const browser=new Window({url:'https://mira.test'});
globalThis.window=browser;globalThis.document=browser.document;globalThis.customElements=browser.customElements;globalThis.getComputedStyle=browser.getComputedStyle.bind(browser);for(const name of ['HTMLElement','Element','Node','CustomEvent','MutationObserver','NodeFilter','HTMLInputElement','Event']) globalThis[name]=browser[name];globalThis.IS_REACT_ACT_ENVIRONMENT=true;
const observers=[];globalThis.ResizeObserver=class{constructor(callback){this.callback=callback;observers.push(this);}observe(element){this.element=element;}disconnect(){}};
const {TurnChanges,ReviewChanges,createAppRuntime,openTurnReview,pullRequestPatch,getRenderablePatch,resolveFileDiffPath,reviewDraftKey}=await bundle(`
export {reviewDraftKey} from './src/state/reviewService';
export {TurnChanges} from './src/components/TurnChanges';
export {ReviewChanges} from './src/components/ReviewChanges';
export {createAppRuntime} from './src/state/createAppRuntime';
export {openTurnReview} from './src/lib/openTurnReview';
export {pullRequestPatch} from './src/lib/pullRequestDiff';
export {getRenderablePatch,resolveFileDiffPath} from './src/lib/diffRender';
`,{
 'src/state/RuntimeProvider.tsx':`import {useSelector} from '@tanstack/react-store'; export const useAppRuntime=()=>globalThis.reviewRuntime; export const useOptionalRuntime=()=>globalThis.reviewRuntime; export function useWorkspaceUiField(key){const runtime=globalThis.reviewRuntime;return [useSelector(runtime.workspaceUi,s=>s[key]),value=>runtime.updateWorkspaceUi(key,value)];}`,
 'src/lib/prefs.ts':`import {useState} from 'react';export const PREF_KEYS={};export const getPref=(key,fallback)=>fallback;export const getBoolPref=(key,fallback)=>fallback;export const useStringPref=(key,fallback)=>useState(fallback);export const useBoolPref=(key,fallback)=>useState(fallback);`,
 'src/lib/diffPrefs.ts':`export const useDiffOptions=()=>({});`,
 'src/components/diffs/ReviewDiffView.tsx':`export function ReviewDiffView({files}){return <div data-testid="review-diff">{files.map(f=><span key={f.filePath}>{f.filePath}</span>)}</div>;}`,
 'src/components/diffs/DiffFileTree.tsx':`export const DiffFileTree=()=>null;`,
});
const summary={text:'repeat prompt',occurrence:1,files:[{path:'src/save.ts',added:3,removed:1,binary:false},{path:'logo.png',added:0,removed:0,binary:true}]};
const flush=()=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,10));});
test('turn files open the right file panel while View changes opens the historical review drawer',async()=>{
 const runtime=createAppRuntime({backend:'https://mira.test',account:'test'});globalThis.reviewRuntime=runtime;
 runtime.turnCommands.hydrate('chat-a',false,[]);
 const files=[],requests=[];globalThis.fetch=async(url,options)=>{requests.push({url,body:JSON.parse(options.body)});return new Response(JSON.stringify({before:'old\n',after:'new\n'}));};
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);const render=element=>root.render(React.createElement(QueryClientProvider,{client:runtime.queryClient},element));
 try {
  await act(()=>render(React.createElement(TurnChanges,{summary,onOpenFile:path=>files.push(path),onReview:()=>openTurnReview(runtime,summary)})));
  await act(()=>host.querySelector('[aria-label="Open file src/save.ts"]').click());assert.deepEqual(files,['src/save.ts']);assert.equal(runtime.workspaceUi.get().reviewOpen,false);
  await act(()=>[...host.querySelectorAll('button')].find(b=>b.textContent.startsWith('View changes')).click());
  assert.equal(runtime.workspaceUi.get().reviewOpen,true);assert.equal(runtime.workspaceUi.get().reviewTurn.session,'chat-a');assert.deepEqual(runtime.workspaceUi.get().reviewTurn.summary,summary);
  await act(()=>render(React.createElement(ReviewChanges,{open:true,onClose(){},onChanged(){},onSendComments(){}})));await flush();
  assert.ok(host.textContent.includes('Review turn changes'));assert.ok(host.querySelector('[data-testid="review-diff"]').textContent.includes('src/save.ts'));
  assert.ok(host.textContent.includes('logo.png'));assert.ok(host.textContent.includes('Binary file'));
  assert.deepEqual(requests,[{url:'/api/sessions/chat-a/turn-diff',body:{text:'repeat prompt',occurrence:1,path:'src/save.ts'}}]);
  assert.ok(![...host.querySelectorAll('button')].some(b=>b.textContent.includes('Revert all')),'historical snapshots cannot discard today’s edits');
 } finally {await act(()=>root.unmount());await runtime.dispose();host.remove();delete globalThis.reviewRuntime;}
});
test('PR hunks use Pierre parsing with real line numbers, rename paths and missing patch handling',()=>{
 const file={filename:'src/new name.ts',previous_filename:'src/old name.ts',status:'renamed',patch:'@@ -40,2 +41,2 @@\n context\n-old\n+new'};
 const patch=pullRequestPatch(file);assert.ok(patch.includes('"a/src/old name.ts"'));
 const parsed=getRenderablePatch(patch,'test');assert.equal(parsed.kind,'files');assert.equal(parsed.files.length,1);assert.equal(resolveFileDiffPath(parsed.files[0]),file.filename);
 assert.equal(parsed.files[0].hunks[0].deletionStart,40);assert.equal(parsed.files[0].hunks[0].additionStart,41);
 assert.equal(pullRequestPatch({...file,patch:null}),null);
});

test('embedded Diff source follows the selected snapshot including the latest repeated prompt',async()=>{
 const runtime=createAppRuntime({backend:'https://mira.test',account:'test'});globalThis.reviewRuntime=runtime;
 runtime.turnCommands.hydrate('chat-a',false,[]);
 const latest={...summary,occurrence:0};
 runtime.updateSessionData('entries',[summary.text,'another',summary.text].map(content=>({kind:'msg',msg:{role:'user',content}})));
 runtime.updateSessionData('turnDiffs',[latest,summary,{text:'another',occurrence:0,files:[]}]);
 globalThis.fetch=async()=>new Response(JSON.stringify({before:'old\n',after:'new\n'}));
 openTurnReview(runtime,latest);
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);const render=element=>root.render(React.createElement(QueryClientProvider,{client:runtime.queryClient},element));
 try {
  await act(()=>render(React.createElement(ReviewChanges,{embedded:true,open:true,onClose(){},onChanged(){},onSendComments(){}})));await flush();
  const selector=host.querySelector('[aria-label="Changes source"]');
  assert.equal(selector.textContent,'Latest turn');
  assert.equal(host.querySelector('.fixed'),null,'embedded review has no modal backdrop');
  const split=host.querySelector('[aria-label="Side by side diff"]');
  assert.equal(split.disabled,true,'narrow pane uses unified diff');
  const observer=observers.at(-1);
  Object.defineProperty(observer.element,'clientWidth',{value:800,configurable:true});
  await act(()=>observer.callback());
  assert.equal(split.disabled,false,'split becomes available when the pane widens');
  const wrap=host.querySelector('[aria-label="Wrap lines"]');
  assert.equal(wrap.getAttribute('aria-pressed'),'false');
  await act(()=>wrap.click());
  assert.equal(wrap.getAttribute('aria-pressed'),'true');
  assert.ok(wrap.className.includes('bg-mira-blue/15'),'active wrap has a distinct fill');
  const numbers=host.querySelector('[aria-label="Line numbers"]');
  assert.equal(numbers.getAttribute('aria-pressed'),'true');
  await act(()=>numbers.click());
  assert.equal(numbers.getAttribute('aria-pressed'),'false');
  assert.ok(!numbers.className.includes('bg-mira-blue/15'),'inactive numbers have no active tint');
  await act(()=>selector.click());await flush();
  assert.equal(selector.getAttribute('aria-expanded'),'true');
  assert.equal(document.querySelector('[data-menu-item="Turn 1"]'),null,'turns live in their own menu');
  await act(()=>document.querySelector('[data-menu-item="Turns"]').click());await flush();
  assert.ok(document.querySelector('[aria-label="Turn changes menu"]'));
  assert.equal(document.querySelector('[data-menu-item="Turn 2"]'),null,'turns without changes are omitted');
  const older=document.querySelector('[data-menu-item="Turn 1"]');
  assert.ok(older,'source menu actually opens');
  await act(()=>older.click());await flush();
  assert.deepEqual(runtime.workspaceUi.get().reviewTurn.summary,summary);
  assert.equal(selector.textContent,'Turn 1');
  assert.equal(selector.getAttribute('aria-expanded'),'false');
  await act(()=>selector.click());await flush();
  await act(()=>document.querySelector('[data-menu-item="Changes"]').click());
  assert.equal(runtime.workspaceUi.get().reviewTurn,null);
 } finally {await act(()=>root.unmount());await runtime.dispose();host.remove();delete globalThis.reviewRuntime;}
});

test('rejected review delivery and source changes preserve drafts until accepted',async()=>{
 const runtime=createAppRuntime({backend:'https://mira.test',account:'drafts'});globalThis.reviewRuntime=runtime;
 runtime.turnCommands.hydrate('chat-a',false,[]);openTurnReview(runtime,summary);
 const scope=runtime.workspaceUi.get().reviewTurn;
 const key=reviewDraftKey('chat-a',summary,scope.turnKey);
 runtime.review.updateDraft(key,[{id:'draft',filePath:'src/save.ts',fileKey:'original',range:{start:1,end:1},rangeLabel:'line 1',code:'original quote',text:'Keep this comment'}]);
 globalThis.fetch=async()=>new Response(JSON.stringify({before:'old\n',after:'new\n'}));
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
 let accepted=false,closed=0;
 const render=()=>root.render(React.createElement(QueryClientProvider,{client:runtime.queryClient},React.createElement(ReviewChanges,{open:true,embedded:true,onClose(){closed++;},onChanged(){},onSendComments(){return accepted;}})));
 const send=()=>[...host.querySelectorAll('button')].find(button=>button.textContent.startsWith('Send 1'));
 try {
  await act(render);await flush();
  await act(()=>send().click());
  assert.equal(closed,0);assert.equal(runtime.review.drafts.get().get(key)[0].text,'Keep this comment');
  assert.match(host.querySelector('[role=alert]').textContent,/not sent/);
  await act(()=>runtime.updateWorkspaceUi('reviewTurn',null));await flush();
  assert.ok(!send());
  await act(()=>runtime.updateWorkspaceUi('reviewTurn',scope));await flush();assert.ok(send());
  await act(()=>root.render(null));await act(render);await flush();assert.ok(send(),'closing/reopening the view retains its draft');
  accepted=true;await act(()=>send().click());assert.equal(closed,1);assert.equal(runtime.review.drafts.get().has(key),false);
 } finally {await act(()=>root.unmount());await runtime.dispose();host.remove();delete globalThis.reviewRuntime;}
});
