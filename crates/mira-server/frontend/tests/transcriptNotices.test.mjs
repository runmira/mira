import assert from 'node:assert/strict';
import { test } from 'node:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { Window } from 'happy-dom';
import { bundle } from './helpers/bundle.mjs';
const window = new Window({ url: 'https://mira.test' });
globalThis.window = window; globalThis.document = window.document;
const { turnActivity, TurnView, AssistantContent, EntryView, StatusLine } = await bundle(`
export { turnActivity } from './src/lib/turnActivity';
export { TurnView } from './src/components/transcript/TurnView';
export { StatusLine } from './src/components/transcript/EntryView';
export { EntryView } from './src/components/transcript/EntryView';
export { AssistantContent } from './src/components/AssistantContent';
`, { 'src/components/diffs/DiffWorkerPoolProvider': `export function DiffWorkerPoolProvider({children}) { return children; }`, 'src/components/ui/Collapse': `export function Collapse({open, children}) { return open ? children : null; }` });
const tool = {kind:'tool',call:{id:'one',function:{name:'read',arguments:'{}'}},preview:null,result:null,status:'complete',agentCall:{id:'one'}};
const answer = {kind:'msg',msg:{role:'assistant',content:'The final answer.'}};
const notice = {kind:'warning',text:'Turn ended manually by user.'};
test('lifecycle notices remain outside folded native and provider work in their original order', () => {
  const second = {kind:'warning',text:'[environment] Switched to local.'};
  for (const fold of [false,true]) {
    const view = turnActivity([tool, notice, answer, second], fold);
    assert.deepEqual(view.notices,[notice,second]);
    assert.equal(view.finalEntry,answer);
    assert.ok(!view.intermediateRaw.includes(notice));
    assert.ok(!view.trailing.some(row=>row.entry===notice));
  }
});
test('a collapsed completed turn still renders interruptions below the reply as a status row', () => {
  const noop=()=>{};
  const html = renderToStaticMarkup(React.createElement(TurnView, {turn:{user:null,body:[tool,answer,notice]},timing:{startedAt:0,endedAt:1000},index:0,expanded:false,onToggle:noop,onDecide:noop,onPlanReply:noop,onOpenAgent:noop}));
  const dom = document.createElement('div');dom.innerHTML=html;
  assert.ok(!dom.querySelector('.transcript-work'));
  assert.equal(dom.querySelector('[data-transcript-notice]')?.textContent.trim(),notice.text);
  assert.ok(html.indexOf('The final answer.')<html.indexOf(notice.text));
});
test('embedded reasoning owns one icon gutter while prose owns the content gutter', () => {
  const html=renderToStaticMarkup(React.createElement(AssistantContent,{transcript:true,text:'<think>Reasoning</think>Answer'}));
  const dom=document.createElement('div');dom.innerHTML=html;
  const button=dom.querySelector('button');
  assert.equal(dom.querySelectorAll('.min-h-7.px-2 svg').length,0,'prose does not acquire a new agent icon');
  assert.ok(button.className.includes('px-2'));
  assert.ok(!button.parentElement.closest('.px-2'), 'thinking is not nested in prose padding');
  assert.ok([...dom.querySelectorAll('.px-2')].some(el=>el.textContent.includes('Answer')));
});

test('intermediate messages have no timestamp or action row; the final reply retains it', () => {
  const noop=()=>{};
  function render(showAssistantActions) {
    const dom=document.createElement('div');
    dom.innerHTML=renderToStaticMarkup(React.createElement(EntryView,{entry:{...answer,msg:{...answer.msg,created_at:0}},showAssistantActions,onDecide:noop,onPlanReply:noop,onOpenAgent:noop}));
    return dom;
  }
  assert.equal(render(false).querySelectorAll('button,time').length,0);
  assert.equal(render(false).textContent,answer.msg.content,'intermediate messages have no secondary timestamp text');
  assert.ok(render(true).querySelector('button'), 'final answer retains actions');
});

test('status rows preserve readable code and detail outside presentational separator roles', () => {
  const dom=document.createElement('div');dom.innerHTML=renderToStaticMarkup(React.createElement(StatusLine,{text:'[environment] Switched to `local`\nEnvironment detail'}));
  const notice=dom.querySelector('[data-transcript-notice]');
  assert.ok(!notice.closest('[role="separator"],[aria-hidden="true"]'));
  assert.equal(notice.querySelector('code').textContent,'local');
  assert.equal(notice.querySelector('pre').textContent,'Environment detail');
});


test('a streaming timeout warning stays a plain status row as responses arrive', () => {
  const warning = {kind:'warning',text:"The agent hasn't responded in 45 seconds. It may be waiting on something outside Mira — a sign-in, a macOS permission or keychain prompt, or the network. Try running it in a terminal to see, or press Stop."};
  const nextAnswer = {kind:'msg',msg:{role:'assistant',content:'Continuing the response.'}};
  const noop = () => {};
  for (const body of [[answer,tool,warning],[answer,tool,warning,nextAnswer]]) {
    const dom = document.createElement('div');
    dom.innerHTML = renderToStaticMarkup(React.createElement(TurnView, {turn:{user:null,body},isActive:true,timing:{startedAt:0,endedAt:null},index:0,expanded:true,onToggle:noop,onDecide:noop,onPlanReply:noop,onOpenAgent:noop}));
    const row = dom.querySelector('[data-transcript-notice]');
    assert.equal(row.textContent,warning.text);
    assert.equal(row.querySelectorAll('.h-px,[role="separator"]').length,0,'warnings do not introduce moving dividers');
    assert.ok(row.classList.contains('items-start'),'wrapped warnings align their icon with the first line');
  }
});

test('steering bubbles remain visible and ordered when native or provider work is collapsed', () => {
  const first = {kind:'msg',msg:{role:'user',content:'First steering instruction',input_intent:'steer'}};
  const second = {kind:'msg',msg:{role:'user',content:'Second steering instruction',input_intent:'steer'}};
  const commentary = {kind:'msg',msg:{role:'assistant',content:'Intermediate commentary'}};
  const noop = () => {};
  for (const native of [false,true]) {
    for (const expanded of [false,true]) {
      const body = [commentary,tool,first,tool,second,{...answer,...(native ? {nativeMessageId:'final',nativePhase:'final_answer'} : {})}];
      const dom = document.createElement('div');
      dom.innerHTML = renderToStaticMarkup(React.createElement(TurnView,{turn:{user:null,body},isActive:false,timing:{startedAt:0,endedAt:1000},index:0,expanded,onToggle:noop,onDecide:noop,onPlanReply:noop,onOpenAgent:noop}));
      assert.ok(dom.textContent.includes(first.msg.content));
      assert.ok(dom.textContent.includes(second.msg.content));
      assert.ok(dom.textContent.indexOf(first.msg.content)<dom.textContent.indexOf(second.msg.content));
      assert.ok(dom.textContent.indexOf(second.msg.content)<dom.textContent.indexOf(answer.msg.content));
      assert.ok(![...dom.querySelectorAll('.transcript-work')].some(row=>row.textContent.includes('steering instruction')));
      assert.equal(dom.textContent.includes(commentary.msg.content),expanded);
    }
  }
});

test('one stop action renders one notice in either event order, retaining denial details', () => {
  const interrupted = {kind:'warning',text:'turn interrupted'};
  const denied = {kind:'warning',text:'turn interrupted (denied 2 pending approval(s))'};
  const other = {kind:'warning',text:'Unrelated warning'};
  const noop = () => {};
  for (const pair of [[interrupted,notice],[notice,interrupted],[notice,notice],[denied,notice],[notice,denied]]) {
    for (const fold of [false,true]) {
      const body = [tool,answer,...pair,other];
      const view = turnActivity(body,fold);
      assert.equal(view.notices.length,2);
      assert.equal(view.notices[0],pair.includes(denied) ? denied : notice);
      assert.equal(view.notices[1],other);
      const dom = document.createElement('div');
      dom.innerHTML = renderToStaticMarkup(React.createElement(TurnView,{turn:{user:null,body},timing:{startedAt:0,endedAt:1000},index:0,expanded:false,onToggle:noop,onDecide:noop,onPlanReply:noop,onOpenAgent:noop}));
      assert.equal(dom.querySelectorAll('[data-transcript-notice]').length,2);
    }
  }
});
