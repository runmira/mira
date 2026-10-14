import assert from 'node:assert/strict';
import {after, test} from 'node:test';
import React,{act} from 'react';
import {createRoot} from 'react-dom/client';
import {Window} from 'happy-dom';
import {bundle} from './helpers/bundle.mjs';
const {CodexAccountPanel}=await bundle("export {CodexAccountPanel} from './src/components/settings/CodexAccountPanel';");
const browser=new Window({url:'https://accounts.mira.test'});
globalThis.window=browser;globalThis.document=browser.document;globalThis.IS_REACT_ACT_ENVIRONMENT=true;
after(()=>browser.happyDOM.abort());
const signedOut={connection_id:'connection-1',phase:'signed_out',account:null,requires_openai_auth:true,requirements:null,requirements_supported:true,login_id:null,auth_url:null,user_code:null,error:null,revision:1};
const pending={...signedOut,phase:'pending',login_id:'login-1',auth_url:'https://auth.openai.com/codex/device',user_code:'ABCD-EFGH',revision:2};
function button(host,text){return [...host.querySelectorAll('button')].find(b=>b.textContent===text);}
async function flush(){await act(async()=>{await new Promise(resolve=>setImmediate(resolve));});}
test('reconnected pending login does not reopen the browser and cancellation is pinned to its owner',async()=>{
  const requests=[],opened=[];browser.open=(url)=>{opened.push(url);};
  globalThis.fetch=async(url,options={})=>{requests.push({url,options});return new Response(JSON.stringify(options.method==='DELETE'?{...signedOut,phase:'cancelled',revision:3}:pending));};
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
  await act(()=>root.render(React.createElement(CodexAccountPanel,{instance:'codex-work',enabled:true,onRefresh(){}})));
  await flush();
  assert.ok(host.textContent.includes('Waiting for sign-in'));
  assert.ok(host.textContent.includes('ABCD-EFGH'));
  assert.deepEqual(opened,[]);
  await act(()=>button(host,'Cancel').click());await flush();
  const cancel=requests.find(r=>r.options.method==='DELETE');
  assert.equal(cancel.url,'/api/engines/codex-work/account/login');
  assert.deepEqual(JSON.parse(cancel.options.body),{connection_id:'connection-1',login_id:'login-1'});
  assert.ok(host.textContent.includes('Sign-in cancelled'));
  assert.ok(!host.textContent.includes('ABCD-EFGH'));
  await act(()=>root.unmount());host.remove();
});
test('a delayed sign-in response cannot open a browser after the selected instance changes',async()=>{
  const opened=[];browser.open=(url)=>{opened.push(url);};let resolveStart;
  globalThis.fetch=async(url,options={})=>options.method==='POST'?new Promise(resolve=>{resolveStart=resolve;}):new Response(JSON.stringify(signedOut));
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
  await act(()=>root.render(React.createElement(CodexAccountPanel,{instance:'codex-work',enabled:true,onRefresh(){}})));await flush();
  await act(()=>button(host,'Sign in with ChatGPT').click());
  assert.ok(resolveStart);
  await act(()=>root.render(React.createElement(CodexAccountPanel,{instance:'codex-personal',enabled:true,onRefresh(){}})));
  resolveStart(new Response(JSON.stringify(pending)));await flush();
  assert.deepEqual(opened,[]);
  assert.ok(!host.textContent.includes('ABCD-EFGH'));
  assert.equal(button(host,'Sign in with ChatGPT').disabled,false);
  await act(()=>root.unmount());host.remove();
});
