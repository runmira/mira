import assert from 'node:assert/strict';
import { test } from 'node:test';
import React, { act } from 'react';
import { createRoot } from 'react-dom/client';
import { Window } from 'happy-dom';
import { bundle } from './helpers/bundle.mjs';
const { VirtualTranscript } = await bundle(`export { VirtualTranscript } from './src/components/VirtualTranscript';`);
const window = new Window({ url: 'https://scroll.mira.test' });
globalThis.window = window; globalThis.document = window.document;
globalThis.sessionStorage = window.sessionStorage;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
const observers = new Set();
let created = 0;
globalThis.ResizeObserver = class {
  targets = new Set();
  constructor(callback) { this.callback = callback; observers.add(this); created++; }
  observe(element) { this.targets.add(element); }
  disconnect() { observers.delete(this); }
};
function rect(top, height) { return { top, bottom: top + height, height, left: 0, right: 600, width: 600 }; }
function fixture() {
  const pane = document.createElement('div'); document.body.append(pane);
  let top = 0;
  Object.defineProperty(pane, 'clientHeight', { value: 600 });
  Object.defineProperty(pane, 'scrollTop', { get: () => top, set: value => { top = Math.max(0, value); } });
  pane.getBoundingClientRect = () => rect(0, 600);
  const root = createRoot(pane);
  const follow = { current: false }, lastInput = { current: Date.now() };
  const heights = new Map();
  const ref = { current: pane };
  function geometry() {
    const column = pane.firstElementChild;
    column.getBoundingClientRect = () => rect(-top, pane.scrollHeight);
    const elements = [...column.children];
    function height(element) { return element.dataset.virtualKey ? heights.get(element.textContent) ?? 280 : Number.parseFloat(element.style.height) || 0; }
    Object.defineProperty(pane, 'scrollHeight', { configurable: true, get: () => [...column.children].reduce((sum, el) => sum + height(el), 0) });
    for (const element of elements) if (element.dataset.virtualKey) element.getBoundingClientRect = () => rect([...element.parentElement.children].slice(0,[...element.parentElement.children].indexOf(element)).reduce((sum, el) => sum + height(el),0) - top, height(element));
    for (const element of elements) if (element.dataset.virtualKey) element.firstElementChild.getBoundingClientRect = () => element.getBoundingClientRect();
  }
  async function render(ids) {
    await act(() => root.render(React.createElement(VirtualTranscript, { pane: ref, identity: 'test', follow, lastInput }, ids.map(id => React.createElement('button', { key: id, 'aria-expanded': false, 'data-transcript-disclosure': '' }, id)))));
    geometry();
  }
  async function resize() {
    await act(() => { for (const observer of [...observers]) observer.callback([...observer.targets].map(target => ({ target }))); });
    geometry();
  }
  return { pane, root, heights, render, resize, geometry, follow };
}
test('row measurement preserves current reading position, including a rapid scroll before resize', async () => {
  const f = fixture(), ids = Array.from({length: 40},(_,i)=>String(i));
  await f.render(ids);
  f.pane.scrollTop = 3000;
  await act(() => f.pane.dispatchEvent(new window.Event('scroll'))); f.geometry();
  f.heights.set('8',500);
  await f.resize();
  assert.equal(f.pane.scrollTop,3220, 'height growth above row 10 preserves its offset');
  // Move without delivering a scroll event: cached visible ranges can lag a fling.
  f.pane.scrollTop = 4500;
  f.heights.set('9',400);
  await f.resize();
  assert.equal(f.pane.scrollTop,4620, 'resize captures current offset, not the previous reading row');
  await act(() => f.root.unmount()); f.pane.remove();
  assert.equal(observers.size,0);
});
test('streaming rerenders keep the row observer attached instead of canceling measurement work', async () => {
  const f = fixture(), ids = Array.from({length: 20},(_,i)=>String(i));
  await f.render(ids);
  await f.resize();
  await f.render(ids);
  const before = created;
  for(let i=0;i<20;i++) await f.render(ids);
  assert.equal(created,before);
  await act(() => f.root.unmount()); f.pane.remove();
});

test('history prepend and a tool disclosure keep the clicked header at its viewport position', async () => {
  const f = fixture(), ids = Array.from({length: 40},(_,i)=>String(i));
  await f.render(ids); await f.resize();
  f.pane.scrollTop = 3000;
  await act(() => f.pane.dispatchEvent(new window.Event('scroll'))); f.geometry(); await f.resize();
  await f.render(['older',...ids]);
  assert.equal(f.pane.scrollTop,3280, 'prepending an estimated row retains the stable reading ID');
  const button = [...f.pane.querySelectorAll('button')].find(el=>el.textContent==='11');
  const before = button.getBoundingClientRect().top;
  await act(() => button.dispatchEvent(new window.MouseEvent('click',{bubbles:true})));
  f.heights.set('8',500);
  await f.resize();
  assert.equal(button.getBoundingClientRect().top,before,'clicked disclosure header stays in place as earlier rows remeasure');
  await act(() => f.root.unmount()); f.pane.remove();
});


test('scroll listeners attach when an ancestor host ref becomes available after child layout effects', async () => {
  const host = document.createElement('div'); document.body.append(host);
  const root = createRoot(host); let pane;
  function Parent() {
    const ref = React.useRef(null);
    return React.createElement('div', {ref: node => {
      ref.current = node;
      if (!node) return;
      pane = node;
      Object.defineProperty(node, 'clientHeight', {configurable:true, value:600});
      node.getBoundingClientRect = () => rect(0,600);
      node.firstElementChild.getBoundingClientRect = () => rect(-node.scrollTop,11200);
    }}, React.createElement(VirtualTranscript, {pane:ref, identity:'ancestor-ref', follow:{current:false}}, Array.from({length:40},(_,i)=>React.createElement('p',{key:i},`Row ${i}`))));
  }
  await act(() => root.render(React.createElement(Parent)));
  pane.scrollTop = 3000;
  await act(() => pane.dispatchEvent(new window.Event('scroll')));
  assert.ok(!pane.querySelector('[data-virtual-key]').textContent.includes('Row 0'), 'scroll updates the mounted range on its first lifecycle');
  assert.ok(pane.textContent.includes('Row 10'));
  await act(() => root.unmount()); host.remove();
});

// Unrelated popovers must not pin their trigger through subsequent row growth.
test('an unrelated expandable control does not pin scrolling or disable following', async () => {
  const f = fixture(), ids = Array.from({length:40},(_,i)=>String(i));
  await f.render(ids); await f.resize();
  f.pane.scrollTop = 3000;
  await act(() => f.pane.dispatchEvent(new window.Event('scroll'))); f.geometry();
  const button = [...f.pane.querySelectorAll('button')].find(el=>el.textContent==='11');
  button.removeAttribute('data-transcript-disclosure');
  f.follow.current = true;
  let triggerTop = button.getBoundingClientRect().top;
  button.getBoundingClientRect = () => rect(triggerTop,20);
  await act(() => button.dispatchEvent(new window.MouseEvent('click',{bubbles:true})));
  assert.equal(f.follow.current,true,'unrelated popovers do not change follow mode');
  triggerTop += 120;
  await f.render(ids);
  assert.equal(f.pane.scrollTop,3000,'a changing popover trigger cannot move the reading position');
  await act(() => f.root.unmount()); f.pane.remove();
});
