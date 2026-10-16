import assert from 'node:assert/strict';
import {after,test} from 'node:test';
import React,{act} from 'react';
import {createRoot} from 'react-dom/client';
import {Window} from 'happy-dom';
import {bundle} from './helpers/bundle.mjs';
const window=new Window({url:'https://mira.test'});
Object.assign(globalThis,{window,document:window.document,IS_REACT_ACT_ENVIRONMENT:true});
after(async()=>window.happyDOM.abort());
const {createAppRuntime,WorkspaceComposer}=await bundle(`
export {createAppRuntime} from './src/state/createAppRuntime';
export {WorkspaceComposer} from './src/app/WorkspaceComposer';`,{
  'src/state/RuntimeProvider':`export const useAppRuntime=()=>globalThis.connectionRuntime;export const useOptionalRuntime=()=>globalThis.connectionRuntime;`,
  'src/components/Composer':`export function Composer(props){globalThis.composerProps=props;return null;}`,
});
test('session attachment does not show reconnecting, while genuine transport loss does',async()=>{
  const runtime=createAppRuntime({backend:'https://mira.test',account:'connection'});
  globalThis.connectionRuntime=runtime;
  runtime.reportConnection('open');runtime.turnCommands.hydrate('codex-a',true,[]);
  runtime.turnCommands.suspend('claude-b');
  const host=document.createElement('div'),root=createRoot(host);
  const context={status:'connecting',sessionId:'claude-b',acpDriver:'claude',acpCommands:[],pendingApprovals:[],pendingPlans:[],pendingQuestions:[],secretRequests:[],infoNotices:[]};
  try{
    await act(()=>root.render(React.createElement(WorkspaceComposer,context)));
    assert.equal(globalThis.composerProps.notices.some(n=>n.id==='connection'),false);
    assert.equal(globalThis.composerProps.disabled,true,'must wait for target Ready');
    assert.equal(runtime.sessionState.get().sessions.get('codex-a').busy,true,'old session remains running');
    await act(()=>runtime.reportConnection('closed'));
    assert.equal(globalThis.composerProps.notices.find(n=>n.id==='connection').title,'Reconnecting to Mira');
    await act(()=>runtime.reportConnection('open'));
    assert.equal(globalThis.composerProps.notices.some(n=>n.id==='connection'),false);
    assert.equal(globalThis.composerProps.disabled,true,'socket recovery alone must not enable wrong-session mutations');
  }finally{await act(()=>root.unmount());runtime.dispose();delete globalThis.connectionRuntime;delete globalThis.composerProps;}
});
