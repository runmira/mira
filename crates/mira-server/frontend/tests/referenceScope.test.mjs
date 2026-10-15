import assert from 'node:assert/strict';
import { test } from 'node:test';
import React, { act } from 'react';
import { createRoot } from 'react-dom/client';
import { Window } from 'happy-dom';
import { bundle } from './helpers/bundle.mjs';
const { RuntimeProvider, useAppRuntime, useReferenceScope } = await bundle(`
export { RuntimeProvider, useAppRuntime } from './src/state/RuntimeProvider';
export { useReferenceScope } from './src/hooks/useReferenceScope';`);
const window=new Window();globalThis.window=window;globalThis.document=window.document;globalThis.IS_REACT_ACT_ENVIRONMENT=true;
test('reference scopes react to chat, cwd and backend/account changes', async()=>{
 let runtime;
 function Probe(){runtime=useAppRuntime();return React.createElement('output',null,useReferenceScope().cacheKey);}
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
 const render=scope=>root.render(React.createElement(RuntimeProvider,{scope},React.createElement(Probe)));
 try {
  await act(()=>render({backend:'https://one.test',account:'alice'}));
  await act(()=>{runtime.turnCommands.hydrate('chat-a',false,[]);runtime.updateSessionData('cwd','/workspace/a');});
  assert.deepEqual(JSON.parse(host.textContent),['https://one.test','alice','chat-a','/workspace/a']);
  await act(()=>{runtime.turnCommands.hydrate('chat-b',false,[]);runtime.updateSessionData('cwd','/workspace/b');});
  assert.deepEqual(JSON.parse(host.textContent),['https://one.test','alice','chat-b','/workspace/b']);
  await act(()=>render({backend:'https://two.test',account:'bob'}));
  assert.deepEqual(JSON.parse(host.textContent),['https://two.test','bob',null,'']);
 } finally {await act(()=>root.unmount());host.remove();}
});
