import assert from 'node:assert/strict';
import { after, test } from 'node:test';
import React, { act } from 'react';
import { createRoot } from 'react-dom/client';
import { Window } from 'happy-dom';
import { bundle } from './helpers/bundle.mjs';

const window = new Window();
globalThis.window = window;
globalThis.document = window.document;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
const observers = [];
globalThis.ResizeObserver = class {
  observed = [];
  disconnected = false;
  constructor(update) { this.update = update; observers.push(this); }
  observe(node) { this.observed.push(node); }
  disconnect() { this.disconnected = true; }
};
let headingBottom = 36;
let rowsTop = 40;
const originalRect = window.HTMLElement.prototype.getBoundingClientRect;
window.HTMLElement.prototype.getBoundingClientRect = function () {
  if (this.hasAttribute('data-sidebar-projects-header')) return { bottom: headingBottom };
  if (this.hasAttribute('data-sidebar-projects-rows')) return { top: rowsTop };
  return originalRect.call(this);
};
after(async () => { window.HTMLElement.prototype.getBoundingClientRect = originalRect; await window.happyDOM.abort(); });
const { ProjectListSection } = await bundle(`export { ProjectListSection } from './src/components/sidebar/ProjectListSection';`);

async function mount(transparent) {
  headingBottom = 36; rowsTop = 40;
  const element = document.createElement('div');
  element.setAttribute('data-sidebar-scroll', '');
  const root = createRoot(element);
  const render = (native, header = 'Projects') => root.render(React.createElement(ProjectListSection, { transparent: native, header }, React.createElement('p', null, 'Session')));
  await act(() => render(transparent));
  return { element, root, render, header: () => element.querySelector('[data-sidebar-projects-header]'), rows: () => element.querySelector('[data-sidebar-projects-rows]') };
}

test('desktop Projects stays sticky and transparent while rows are clipped beneath it', async () => {
  const h = await mount(true);
  try {
    assert.ok(h.header().classList.contains('sticky'));
    assert.ok(h.header().classList.contains('top-0'));
    assert.ok(h.header().classList.contains('bg-transparent'));
    assert.ok(!h.header().classList.contains('bg-panel'));
    assert.equal(h.rows().style.getPropertyValue('--sidebar-projects-clip'), '0px');
    rowsTop = -60;
    await act(() => h.element.dispatchEvent(new window.Event('scroll')));
    assert.equal(h.rows().style.getPropertyValue('--sidebar-projects-clip'), '96px');
    headingBottom = 64;
    await act(() => observers.at(-1).update());
    assert.equal(h.rows().style.getPropertyValue('--sidebar-projects-clip'), '124px', 'selection toolbar resize keeps rows clear');
    rowsTop = 68;
    await act(() => h.element.dispatchEvent(new window.Event('scroll')));
    assert.equal(h.rows().style.getPropertyValue('--sidebar-projects-clip'), '0px', 'scrolling back reveals every row');
  } finally { await act(() => h.root.unmount()); }
  assert.equal(observers.at(-1).disconnected, true);
});

test('layout changes above Projects recalculate native clipping without a scroll', async () => {
  const h = await mount(true);
  try {
    rowsTop = -20;
    await act(() => h.render(true, 'Projects — selection'));
    assert.equal(h.rows().style.getPropertyValue('--sidebar-projects-clip'), '56px');
    assert.ok(observers.at(-1).observed.includes(h.element));
  } finally { await act(() => h.root.unmount()); }
});

test('web Projects keeps its sticky panel and needs no native clipping observer', async () => {
  const count = observers.length;
  const h = await mount(false);
  try {
    assert.ok(h.header().classList.contains('sticky'));
    assert.ok(h.header().classList.contains('bg-panel'));
    assert.equal(h.rows().style.clipPath, '');
    assert.equal(observers.length, count);
  } finally { await act(() => h.root.unmount()); }
});
