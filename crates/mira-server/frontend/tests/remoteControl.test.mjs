import assert from 'node:assert/strict';
import { test } from 'node:test';
import React, { act } from 'react';
import { createRoot } from 'react-dom/client';
import { Window } from 'happy-dom';
import { bundle } from './helpers/bundle.mjs';
const { COMMANDS, slashState, agentToolAsMira, workspaceHasContent } = await bundle(`
export { COMMANDS, slashState } from './src/components/commands';
export { agentToolAsMira } from './src/lib/agentTools';
export { workspaceHasContent } from './src/components/context/WorkspaceSection';`);

test('remote control commands stay local and validate on/off arguments', () => {
  const calls=[]; const ctx={onRemoteControl: enabled=>calls.push(enabled)};
  for(const input of ['/remote control','/remote control on','/remote control off']) {
    const parsed=slashState(input); assert.equal(parsed.mode,'args');
    assert.equal(parsed.command.run(parsed.args,ctx),undefined);
  }
  assert.deepEqual(calls,[true,true,false]);
  const overridden=slashState('/remote control off',[{name:'remote',isCustom:true}]);
  assert.equal(overridden.command.isCustom,undefined,'agent commands cannot take over machine access');
  const remote=COMMANDS.find(command=>command.name==='remote');
  assert.throws(()=>remote.run('control maybe',ctx),/usage/);
  assert.throws(()=>remote.run('another-computer',ctx),/usage/);
  assert.deepEqual(calls,[true,true,false]);
});

const {latestEditedTurn}=await bundle(`export {latestEditedTurn} from './src/components/context/LastTurnChanges';`,{
  'src/components/TurnChanges':'export function TurnChanges(){return null;}',
});
test('latest saved edits follow transcript order and repeated prompts after commits',()=>{
  const entries=['fix','second','fix','pending'].map(content=>({kind:'msg',msg:{role:'user',content}}));
  const summaries=[{text:'fix',occurrence:1,files:[{path:'first/a.rs'}]},{text:'fix',occurrence:0,files:[{path:'second/b.rs'}]},{text:'second',occurrence:0,files:[{path:'first/c.rs'}]}];
  assert.equal(latestEditedTurn(entries,summaries).files[0].path,'second/b.rs');
  assert.equal(latestEditedTurn(entries,[]),null);
});

test('Codex multi-file changes stay patches and parent workspaces expose changes', () => {
  const changes=[{path:'/workspace/mira/a.rs',diff:'@@ -1 +1 @@\n-old\n+new'},{path:'/workspace/mira/b.rs',diff:'@@ -0,0 +1 @@\n+created'}];
  const tool=agentToolAsMira({id:'edit',name:'fileChange',kind:'edit',status:'completed',raw_input:{path:changes[0].path,changes,patch:'unified hunks'},locations:changes.map(({path})=>({path})),content:[]});
  assert.equal(tool.name,'apply_patch');assert.deepEqual(tool.args.changes,changes);
  assert.equal(tool.args.patch,'unified hunks');assert.equal(tool.args.content,undefined);
  assert.equal(workspaceHasContent({in_repo:false},{files:[{path:'mira/a.rs'}]},null,false),true);
  assert.equal(workspaceHasContent({in_repo:false},{files:[]},null,false),false);
  assert.equal(workspaceHasContent(null,{files:[]},{number:213},false),true,'a PR can appear before Git status resolves');
});

const {RemoteControlSection,setRemoteFixture}=await bundle(`
export {RemoteControlSection} from './src/components/context/RemoteControlSection';
export {setRemoteFixture} from './src/hooks/useRemoteControl';`,{
  'src/hooks/useRemoteControl': `let fixture; export function setRemoteFixture(next){fixture=next;} export function useRemoteControl(){return fixture;}`,
  'src/lib/remote': `export const remoteAccessAvailable=()=>true;`,
  'src/components/ui/Menu': `export function Menu({open,children}){return open?<div role="menu">{children}</div>:null;} export function MenuItem({label,onSelect}){return <button role="menuitem" onClick={onSelect}>{label}</button>;}`,
});
const window=new Window();globalThis.window=window;globalThis.document=window.document;globalThis.IS_REACT_ACT_ENVIRONMENT=true;
Object.defineProperty(globalThis,'navigator',{value:window.navigator,configurable:true});

test('remote toggle stays compact without a connected indicator or links and never enables on mount',async()=>{
 const calls=[];
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
 const remote={enabled:false,supported:true,status:'off',hostname:null};
 const fixture={remote,busy:false,error:null,setEnabled:async enabled=>calls.push(enabled)};
 try {
  setRemoteFixture(fixture);await act(()=>root.render(React.createElement(RemoteControlSection)));
  assert.deepEqual(calls,[]);assert.equal(host.querySelector('[aria-label="Remote control options"]'),null);
  await act(()=>host.querySelector('[role=switch]').click());assert.deepEqual(calls,[true]);
  setRemoteFixture({...fixture,remote:{...remote,enabled:true,status:'connected',hostname:'m-test.runmira.dev'}});
  await act(()=>root.render(React.createElement(RemoteControlSection)));
  assert.equal(host.querySelector('[role=switch]').getAttribute('aria-checked'),'true');
  assert.equal(host.querySelector('[aria-label="Remote control options"]'),null);
  assert.equal(host.querySelector('a'),null);
  assert.ok(!host.textContent.includes('Connected'));
  assert.ok(!host.textContent.includes('m-test.runmira.dev'));
  setRemoteFixture({...fixture,busy:true,error:new Error('Sign in required')});
  await act(()=>root.render(React.createElement(RemoteControlSection)));
  assert.equal(host.querySelector('[role=switch]').disabled,true);assert.match(host.querySelector('[role=alert]').textContent,/Sign in required/);
 } finally {await act(()=>root.unmount());host.remove();}
});
