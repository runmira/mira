import assert from 'node:assert/strict';
import { test } from 'node:test';
import React, { act, StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { Window } from 'happy-dom';
import { bundle } from './helpers/bundle.mjs';
const { createSummaryPlayback, LiveOutputPanel, usePacedSummary } = await bundle(`
export { createSummaryPlayback } from './src/lib/summaryPlayback';
export { LiveOutputPanel } from './src/components/tools/ToolResult';
export { usePacedSummary } from './src/lib/usePacedSummary';
`);
const window=new Window({url:'https://mira.test'});
globalThis.window=window;globalThis.document=window.document;globalThis.IS_REACT_ACT_ENVIRONMENT=true;
const snapshot=(key,version=key)=>({key,version,value:version});
function timerFixture() {
  let now=0, id=0;const jobs=new Map();
  return { jobs, timing:{now:()=>now,schedule:(fn,delay)=>{const key=++id;jobs.set(key,{fn,at:now+delay});return key;},cancel:key=>jobs.delete(key)},
    tick(delta){now+=delta;for(const [key,job] of [...jobs]) if(job.at<=now){jobs.delete(key);job.fn();}} };
}
test('tool summaries bound pending updates, refresh by tool identity and immediately converge on completion',()=>{
  const clock=timerFixture(),seen=[];
  const player=createSummaryPlayback(snapshot('a'),item=>seen.push(item.value),clock.timing);
  player.update(snapshot('b'),true);
  player.update(snapshot('c'),true);player.update(snapshot('d'),true);player.update(snapshot('e'),true);
  player.update(snapshot('c','c newest metadata'),true);
  assert.deepEqual(seen,['b']);
  clock.tick(800);assert.equal(seen.at(-1),'c newest metadata');
  clock.tick(800);assert.equal(seen.at(-1),'e');assert.ok(!seen.includes('d'));
  player.update(snapshot('e','e refreshed in place'),true);assert.equal(seen.at(-1),'e refreshed in place');
  player.update(snapshot('f'),true);player.update(snapshot('g'),true);
  player.update(snapshot('completed'),false);assert.equal(seen.at(-1),'completed');assert.equal(clock.jobs.size,0);
  player.dispose();player.update(snapshot('late'),true);assert.equal(seen.at(-1),'completed');
});
test('a delayed timer skips stale summaries and disposal cancels pending work',()=>{
  const clock=timerFixture(),seen=[];const player=createSummaryPlayback(snapshot('a'),item=>seen.push(item.value),clock.timing);
  player.update(snapshot('b'),true);player.update(snapshot('c'),true);player.update(snapshot('d'),true);
  clock.tick(1200);assert.deepEqual(seen,['b','d']);
  player.dispose();assert.equal(clock.jobs.size,0);clock.tick(10000);assert.deepEqual(seen,['b','d']);
});
test('output freezes the reading snapshot across streaming and completion, then resumes the newest buffer',async()=>{
  const node=document.createElement('div');const root=createRoot(node);
  let lines=Array.from({length:40},(_,i)=>`line ${i}`);
  function render(running=true){return root.render(React.createElement(LiveOutputPanel,{lines,running,expanded:true}));}
  await act(()=>render());
  const viewport=node.querySelector('[data-live-output]');
  Object.defineProperty(viewport,'scrollHeight',{value:1000,configurable:true});
  Object.defineProperty(viewport,'clientHeight',{value:200,configurable:true});
  lines=[...lines,'line 40'];await act(()=>render());
  viewport.scrollTop=500;await act(()=>viewport.dispatchEvent(new window.Event('scroll')));
  assert.equal(viewport.dataset.following,'false');
  lines=['new buffer after truncation','newest output'];await act(()=>render(false));
  assert.equal(viewport.scrollTop,500);
  assert.ok(node.textContent.includes('line 0'));assert.ok(!node.textContent.includes('newest output'));
  const jump=[...node.querySelectorAll('button')].find(el=>el.textContent==='Jump to live');
  await act(()=>jump.click());
  assert.equal(viewport.dataset.following,'true');assert.ok(node.textContent.includes('newest output'));assert.ok(!node.textContent.includes('line 0'));
  await act(()=>root.unmount());
});
test('returning to the bottom resumes output and historical output does not auto-follow',async()=>{
  const node=document.createElement('div');const root=createRoot(node);
  const lines=Array.from({length:40},(_,i)=>`line ${i}`);
  await act(()=>root.render(React.createElement(LiveOutputPanel,{lines,running:false,expanded:true})));
  const viewport=node.querySelector('[data-live-output]');assert.equal(viewport.scrollTop,0);
  Object.defineProperty(viewport,'scrollHeight',{value:1000});Object.defineProperty(viewport,'clientHeight',{value:200});
  await act(()=>root.render(React.createElement(LiveOutputPanel,{lines:[...lines,'tail'],running:true,expanded:true})));
  viewport.scrollTop=500;await act(()=>viewport.dispatchEvent(new window.Event('scroll')));
  assert.equal(viewport.dataset.following,'false');
  viewport.scrollTop=800;await act(()=>viewport.dispatchEvent(new window.Event('scroll')));
  assert.equal(viewport.dataset.following,'true');await act(()=>root.unmount());
});
test('summary pacing recreates its lifecycle under Strict Mode and completion clears pending display',async()=>{
  const node=document.createElement('div');const root=createRoot(node);
  function Summary({id,active}){const value=usePacedSummary(snapshot(id),active);return React.createElement('span',null,value.value);}
  await act(()=>root.render(React.createElement(StrictMode,null,React.createElement(Summary,{id:'a',active:true}))));
  await act(()=>root.render(React.createElement(StrictMode,null,React.createElement(Summary,{id:'b',active:true}))));assert.equal(node.textContent,'b');
  await act(()=>root.render(React.createElement(StrictMode,null,React.createElement(Summary,{id:'c',active:true}))));assert.equal(node.textContent,'b');
  await act(()=>root.render(React.createElement(StrictMode,null,React.createElement(Summary,{id:'completed',active:false}))));assert.equal(node.textContent,'completed');
  await act(()=>root.unmount());
});
