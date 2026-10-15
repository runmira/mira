import assert from 'node:assert/strict';
import {test} from 'node:test';
import React,{act} from 'react';
import {createRoot} from 'react-dom/client';
import {Window} from 'happy-dom';
import {bundle} from './helpers/bundle.mjs';
const browser=new Window({url:'https://mira.test'});
globalThis.window=browser;globalThis.document=browser.document;globalThis.IS_REACT_ACT_ENVIRONMENT=true;
const {createAppRuntime,useResource,QueryClientProvider,focusManager}=await bundle(`
 export {createAppRuntime} from './src/state/createAppRuntime';
 export {useResource} from './src/state/resources/hooks';
 export {QueryClientProvider,focusManager} from '@tanstack/react-query';
`,{'src/state/RuntimeProvider.tsx':'export const useAppRuntime=()=>globalThis.repositoryRuntime;export const useOptionalRuntime=()=>null;'});
const flush=()=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,30));});
test('context changes poll during generic tools, refresh on focus, and stop when disconnected',async()=>{
 const runtime=createAppRuntime({backend:'https://mira.test',account:'test'});globalThis.repositoryRuntime=runtime;
 runtime.turnCommands.hydrate('chat-a',true,[]);runtime.reportConnection('open');
 let changed=false;const requests=[];
 globalThis.fetch=async(url,options)=>{
  requests.push({url,session:options.headers['X-Mira-Session']});
  return new Response(JSON.stringify({added:changed?1:0,removed:0,files:changed?[{path:'shell-edit.ts',status:'modified',added:1,removed:0,binary:false}]:[]}));
 };
 function Probe(){const result=useResource('sessionDiff');return React.createElement('div',null,result.data?.files.map(f=>f.path).join(',')??'loading');}
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
 try {
  await act(()=>root.render(React.createElement(QueryClientProvider,{client:runtime.queryClient},React.createElement(Probe))));await flush();
  assert.equal(host.textContent,'');
  changed=true;
  await act(async()=>{await new Promise(resolve=>setTimeout(resolve,3200));});await flush();
  assert.equal(host.textContent,'shell-edit.ts','files appear while the tool is still running');
  assert.ok(requests.every(r=>r.session==='chat-a'));
  await act(()=>runtime.turnCommands.reportTurnRunning(false));
  changed=false;
  await act(()=>{focusManager.setFocused(false);focusManager.setFocused(true);});await flush();
  assert.equal(host.textContent,'','focus refreshes even a recently fetched result');
  await act(()=>runtime.reportConnection('closed'));
  const before=requests.length;
  await act(async()=>{await new Promise(resolve=>setTimeout(resolve,3200));});
  assert.equal(requests.length,before,'disconnected queries cannot keep polling');
 } finally {
  await act(()=>root.unmount());await runtime.dispose();host.remove();focusManager.setFocused(undefined);await browser.happyDOM.abort();delete globalThis.repositoryRuntime;
 }
});
