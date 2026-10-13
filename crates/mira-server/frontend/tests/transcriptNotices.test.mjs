import assert from 'node:assert/strict';
import { test } from 'node:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { Window } from 'happy-dom';
import { bundle } from './helpers/bundle.mjs';
const window = new Window({ url: 'https://mira.test' });
globalThis.window = window; globalThis.document = window.document;
const { turnActivity, TurnView, AssistantContent, EntryView } = await bundle(`
export { turnActivity } from './src/lib/turnActivity';
export { TurnView } from './src/components/transcript/TurnView';
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
test('a collapsed completed turn still renders interruptions below the reply as a divider', () => {
  const noop=()=>{};
  const html = renderToStaticMarkup(React.createElement(TurnView, {turn:{user:null,body:[tool,answer,notice]},timing:{startedAt:0,endedAt:1000},index:0,expanded:false,onToggle:noop,onDecide:noop,onPlanReply:noop,onOpenAgent:noop}));
  const dom = document.createElement('div');dom.innerHTML=html;
  assert.ok(!dom.querySelector('.transcript-work'));
  assert.equal(dom.querySelector('[role="separator"]')?.textContent.trim(),notice.text);
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
