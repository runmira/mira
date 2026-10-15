import assert from 'node:assert/strict';
import {test} from 'node:test';
import React,{act} from 'react';
import {createRoot} from 'react-dom/client';
import {Window} from 'happy-dom';
import {bundle} from './helpers/bundle.mjs';
const browser=new Window({url:'https://mira.test'});
globalThis.window=browser;globalThis.document=browser.document;globalThis.customElements=browser.customElements;globalThis.getComputedStyle=browser.getComputedStyle.bind(browser);for(const name of ['HTMLElement','Element','Node','CustomEvent','MutationObserver','NodeFilter','HTMLInputElement','Event']) globalThis[name]=browser[name];globalThis.IS_REACT_ACT_ENVIRONMENT=true;
const observers=[];globalThis.ResizeObserver=class{constructor(callback){this.callback=callback;observers.push(this);}observe(element){this.element=element;}disconnect(){}};
const {TurnChanges,ReviewChanges,createAppRuntime,openTurnReview,pullRequestPatch,getRenderablePatch,resolveFileDiffPath}=await bundle(`
export {TurnChanges} from './src/components/TurnChanges';
export {ReviewChanges} from './src/components/ReviewChanges';
export {createAppRuntime} from './src/state/createAppRuntime';
export {openTurnReview} from './src/lib/openTurnReview';
export {pullRequestPatch} from './src/lib/pullRequestDiff';
export {getRenderablePatch,resolveFileDiffPath} from './src/lib/diffRender';
`,{
 'src/state/RuntimeProvider.tsx':`import {useSelector} from '@tanstack/react-store'; export const useAppRuntime=()=>globalThis.reviewRuntime; export const useOptionalRuntime=()=>globalThis.reviewRuntime; export function useWorkspaceUiField(key){const runtime=globalThis.reviewRuntime;return [useSelector(runtime.workspaceUi,s=>s[key]),value=>runtime.updateWorkspaceUi(key,value)];}`,
 'src/lib/prefs.ts':`export const PREF_KEYS={};export const getPref=(key,fallback)=>fallback;export const getBoolPref=(key,fallback)=>fallback;export const useStringPref=(key,fallback)=>[fallback,()=>{}];export const useBoolPref=(key,fallback)=>[fallback,()=>{}];`,
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
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
 try {
  await act(()=>root.render(React.createElement(TurnChanges,{summary,onOpenFile:path=>files.push(path),onReview:()=>openTurnReview(runtime,summary)})));
  await act(()=>host.querySelector('[aria-label="Open file src/save.ts"]').click());assert.deepEqual(files,['src/save.ts']);assert.equal(runtime.workspaceUi.get().reviewOpen,false);
  await act(()=>[...host.querySelectorAll('button')].find(b=>b.textContent.startsWith('View changes')).click());
  assert.equal(runtime.workspaceUi.get().reviewOpen,true);assert.deepEqual(runtime.workspaceUi.get().reviewTurn,{session:'chat-a',summary});
  await act(()=>root.render(React.createElement(ReviewChanges,{open:true,onClose(){},onChanged(){},onSendComments(){}})));await flush();
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
 runtime.updateSessionData('turnDiffs',[latest,summary]);
 globalThis.fetch=async()=>new Response(JSON.stringify({before:'old\n',after:'new\n'}));
 openTurnReview(runtime,latest);
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
 try {
  await act(()=>root.render(React.createElement(ReviewChanges,{embedded:true,open:true,onClose(){},onChanged(){},onSendComments(){}})));await flush();
  const selector=host.querySelector('[aria-label="Changes source"]');
  assert.equal(selector.textContent,'Latest turn');
  assert.equal(host.querySelector('.fixed'),null,'embedded review has no modal backdrop');
  const split=host.querySelector('[aria-label="Side by side diff"]');
  assert.equal(split.disabled,true,'narrow pane uses unified diff');
  const observer=observers.at(-1);
  Object.defineProperty(observer.element,'clientWidth',{value:800,configurable:true});
  await act(()=>observer.callback());
  assert.equal(split.disabled,false,'split becomes available when the pane widens');
  await act(()=>selector.click());await flush();
  assert.equal(selector.getAttribute('aria-expanded'),'true');
  const older=[...document.querySelectorAll('[role="menuitem"]')].find(item=>item.dataset.menuItem?.startsWith('Turn edits'));
  assert.ok(older,'source menu actually opens');
  await act(()=>older.click());await flush();
  assert.deepEqual(runtime.workspaceUi.get().reviewTurn.summary,summary);
  assert.ok(selector.textContent.startsWith('Turn edits'));
  assert.equal(selector.getAttribute('aria-expanded'),'false');
  await act(()=>selector.click());await flush();
  await act(()=>document.querySelector('[data-menu-item="Changes"]').click());
  assert.equal(runtime.workspaceUi.get().reviewTurn,null);
 } finally {await act(()=>root.unmount());await runtime.dispose();host.remove();delete globalThis.reviewRuntime;}
});
