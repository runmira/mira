import assert from 'node:assert/strict';
import { after, test } from 'node:test';
import React, { act, useRef } from 'react';
import { createRoot } from 'react-dom/client';
import { createMemoryRouter, Outlet, RouterProvider } from 'react-router';
import { useSelector } from '@tanstack/react-store';
import { Window } from 'happy-dom';
import { bundle } from './helpers/bundle.mjs';

const window = new Window({ url: 'https://mira.test' });
globalThis.window = window;
globalThis.document = window.document;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
after(async () => { await window.happyDOM.abort(); });

const { createAppRuntime, useSessionCommands, useAppNavigation, useChatRoute } = await bundle(`
export { createAppRuntime } from './src/state/createAppRuntime';
export { useSessionCommands } from './src/app/useSessionCommands';
export { useAppNavigation } from './src/app/useAppNavigation';
export { useChatRoute } from './src/app/useHistoryNavigation';
`, {
  'src/state/RuntimeProvider': 'export const useAppRuntime = () => globalThis.navigationRuntime; export const useOptionalRuntime = () => globalThis.navigationRuntime ?? null;',
  'src/lib/attention': 'export const callForAttention = () => {};',
});

for (const page of ['pull-request', 'plugins']) {
  for (const target of ['active', 'other']) {
    for (const connected of [true, false]) {
      test(`${page}: picking ${target === 'active' ? 'the current' : 'another'} chat opens it immediately ${connected ? 'before Ready' : 'while disconnected'}`, async () => {
        const runtime = createAppRuntime({ backend: 'https://mira.test', account: 'sidebar' });
        globalThis.navigationRuntime = runtime;
        runtime.turnCommands.hydrate('active', false, []);
        const attached = [];
        function Root() {
          const navigation = useAppNavigation();
          const sessionId = useSelector(runtime.sessionState, state => state.selected ?? state.active ?? '');
          const commands = useSessionCommands({
            ...navigation,
            getSessionId: () => runtime.sessionState.get().selected ?? runtime.sessionState.get().active,
            historyRequestRef: useRef(null),
            recordHistoryLoading() {},
            thinkingIdleTimerRef: useRef(null),
            attachFromRef: useRef(null),
            turnCommands: runtime.turnCommands,
            wsRef: useRef(connected ? { attach: id => attached.push(id) } : null),
          });
          useChatRoute(navigation.mainView, sessionId, commands.attachSession);
          return React.createElement(React.Fragment, null,
            React.createElement('button', { onClick: () => runtime.transition(() => commands.attachSession(target)) }, 'Pick session'),
            React.createElement(Outlet));
        }
        const router = createMemoryRouter([{ path: '/', Component: Root, children: [
          { path: 'pull-request', element: React.createElement('p', null, 'Pull requests') },
          { path: 'plugins', element: React.createElement('p', null, 'Plugins') },
          { path: 'chat', element: React.createElement('p', null, 'Chat') },
          { path: 'chat/:id', element: React.createElement('p', null, 'Selected chat') },
        ] }], { initialEntries: [`/${page}`] });
        const element = document.createElement('div');
        const root = createRoot(element);
        try {
          await act(() => root.render(React.createElement(RouterProvider, { router })));
          assert.equal(router.state.location.pathname, `/${page}`, 'initial mount preserves the page');
          await act(() => element.querySelector('button').click());
          assert.equal(router.state.location.pathname, `/chat/${target}`);
          assert.equal(runtime.sessionState.get().selected, target);
          assert.deepEqual(attached, connected ? [target] : []);
          await act(() => router.navigate(-1));
          assert.equal(router.state.location.pathname, `/${page}`, 'Back restores the page');
          await act(() => router.navigate(1));
          assert.equal(router.state.location.pathname, `/chat/${target}`, 'Forward restores the picked chat');
        } finally {
          await act(() => root.unmount());
          router.dispose();
          await runtime.dispose();
          delete globalThis.navigationRuntime;
        }
      });
    }
  }
}
