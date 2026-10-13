import assert from 'node:assert/strict';
import { test } from 'node:test';
import React, { act } from 'react';
import { createRoot } from 'react-dom/client';
import { Window } from 'happy-dom';
import { createStore, useSelector } from '@tanstack/react-store';
import { bundle } from './helpers/bundle.mjs';
const { createAppRuntime, RuntimeProvider, useAppRuntime, useContextPanelState } = await bundle(`
export { createAppRuntime } from './src/state/createAppRuntime';
export { RuntimeProvider, useAppRuntime, useContextPanelState } from './src/state/RuntimeProvider';
`);
const scope = { backend: 'https://mira.test', account: 'a' };

test('runtime instances isolate UI and resource caches, disposal cancels work and blocks commands', async () => {
  const writes = [];
  const a = createAppRuntime(scope, { getItem: () => '1', setItem: (...args) => writes.push(args) });
  const b = createAppRuntime({ ...scope, account: 'b' });
  a.commands.setContextOpen(false);
  assert.equal(b.workspaceUi.get().contextOpen, false);
  assert.deepEqual(writes, [['mira.context.open', '0']]);
  assert.notDeepEqual(a.resourceKey('session', 'same'), b.resourceKey('session', 'same'));
  a.queryClient.setQueryData(a.resourceKey('session', 'same'), { text: 'private' });
  assert.equal(b.queryClient.getQueryData(b.resourceKey('session', 'same')), undefined);
  let signal;
  const pending = a.queryClient.fetchQuery({ queryKey: a.resourceKey('pending'), queryFn: ({ signal: s }) => {
    signal = s;
    return new Promise(() => {});
  }}).catch(() => undefined);
  a.dispose();
  a.dispose();
  await pending;
  assert.equal(signal.aborted, true);
  assert.equal(a.queryClient.getQueryCache().getAll().length, 0);
  a.commands.setContextOpen(true);
  assert.equal(a.workspaceUi.get().contextOpen, false);
  b.dispose();
});

test('malformed or unavailable UI storage falls back safely', () => {
  for (const storage of [{ getItem: () => '{broken', setItem() {} }, {
    getItem() { throw Error('denied'); }, setItem() { throw Error('denied'); },
  }]) {
    const runtime = createAppRuntime(scope, storage);
    assert.equal(runtime.workspaceUi.get().contextOpen, false);
    runtime.commands.setContextOpen(true);
    assert.equal(runtime.workspaceUi.get().contextOpen, true);
    runtime.dispose();
  }
});

const window = new Window({ url: scope.backend });
globalThis.window = window;
globalThis.document = window.document;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

test('installed selector ignores 100 unrelated transitions and unsubscribes on unmount', async () => {
  const store = createStore({ open: false, tokens: '' });
  let renders = 0;
  let subscriptions = 0;
  const source = { get: () => store.get(), subscribe(listener) {
    subscriptions++;
    const sub = store.subscribe(listener);
    return { unsubscribe() { subscriptions--; sub.unsubscribe(); } };
  }};
  function Probe() { const open = useSelector(source, s => s.open); renders++; return String(open); }
  const root = createRoot(document.createElement('div'));
  await act(() => root.render(React.createElement(Probe)));
  const before = renders;
  await act(() => { for (let i = 0; i < 100; i++) store.setState(s => ({ ...s, tokens: s.tokens + 'x' })); });
  assert.equal(renders, before);
  await act(() => store.setState(s => ({ ...s, open: true })));
  assert.equal(renders, before + 1);
  await act(() => root.unmount());
  assert.equal(subscriptions, 0);
});

test('provider survives Strict Mode, replaces account runtime, and clears caches on unmount', async () => {
  let runtime;
  let open;
  function Probe() { runtime = useAppRuntime(); [open] = useContextPanelState(); return String(open); }
  const root = createRoot(document.createElement('div'));
  const render = account => React.createElement(React.StrictMode, null,
    React.createElement(RuntimeProvider, { scope: { ...scope, account } }, React.createElement(Probe)));
  await act(() => root.render(render('a')));
  const old = runtime;
  await act(() => runtime.commands.setContextOpen(true));
  assert.equal(open, true);
  old.queryClient.setQueryData(old.resourceKey('private'), 'private');
  await act(() => root.render(render('b')));
  assert.notEqual(runtime, old);
  assert.equal(runtime.scope.account, 'b');
  assert.equal(old.queryClient.getQueryCache().getAll().length, 0);
  old.commands.setContextOpen(false);
  assert.equal(old.workspaceUi.get().contextOpen, true);
  const current = runtime;
  current.queryClient.setQueryData(current.resourceKey('private'), 'private');
  await act(() => root.unmount());
  assert.equal(current.queryClient.getQueryCache().getAll().length, 0);
});

test('activity selectors ignore UI changes and revision-only snapshots', async () => {
  const runtime = createAppRuntime(scope);
  const snapshot = (revision, running) => ({ snapshot: { epoch: 'test', revision, running } });
  runtime.commands.applySessionActivity(snapshot(1, ['a', 'b']));
  let renders = 0;
  function Probe() {
    const running = useSelector(runtime.sessionActivity, state => state.activity?.running);
    renders++;
    return String(running?.size);
  }
  const root = createRoot(document.createElement('div'));
  await act(() => root.render(React.createElement(Probe)));
  const before = renders;
  await act(() => {
    runtime.commands.setContextOpen(true);
    for (let i = 2; i <= 100; i++) runtime.commands.applySessionActivity(snapshot(i, ['a', 'b']));
  });
  assert.equal(renders, before);
  await act(() => runtime.commands.applySessionActivity(snapshot(101, ['b'])));
  assert.equal(renders, before + 1);
  await act(() => root.unmount());
  await runtime.dispose();
});
