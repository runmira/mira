import assert from 'node:assert/strict';
import { test } from 'node:test';
import React, { act, useMemo } from 'react';
import { createRoot } from 'react-dom/client';
import { Window } from 'happy-dom';
import { bundle } from './helpers/bundle.mjs';
const { RuntimeProvider, useAppRuntime, useSessionField, useLiveEntries, WorkspaceComposer, appendToken } = await bundle(`
export { RuntimeProvider, useAppRuntime, useSessionField, useLiveEntries } from './src/state/RuntimeProvider';
export { WorkspaceComposer } from './src/app/WorkspaceComposer';
export { appendToken } from './src/transcript/entries';
`, { 'src/components/Composer': `export function Composer() { globalThis.composerRenders++; return null; }` });
const window = new Window({ url: 'https://mira.test' });
globalThis.window = window; globalThis.document = window.document; globalThis.IS_REACT_ACT_ENVIRONMENT = true;

test('streaming wakes the transcript boundary while the shell and actual composer boundary remain isolated', async () => {
  let runtime; let shellRenders = 0; let streamRenders = 0; let outline;
  globalThis.composerRenders = 0;
  function Pane({ props }) {
    const entries = useLiveEntries(); streamRenders++;
    return React.createElement('div', null, entries.at(-1)?.msg?.content,
      React.createElement(WorkspaceComposer, props));
  }
  function Shell() {
    runtime = useAppRuntime(); [outline] = useSessionField('entries'); shellRenders++;
    const props = useMemo(() => ({
      ctxReserve: false, restoreNote: null, status: 'open', busy: false, mode: 'manual', model: 'test', cwd: '',
      sessionId: 'a', queuedFor: () => [], engine: null, engines: [], acpAgents: [], acpDriver: null, acpConfig: [],
      acpDescriptors: null, acpModes: null, agentPostures: [], pendingApprovals: [], pendingPlans: [], pendingQuestions: [],
      secretRequests: [], infoNotices: [], commands: [], acpCommands: [], skills: [], usageRing: {},
      wsRef: runtime.service.refs.wsRef, completedUpdatesRef: runtime.service.refs.completedUpdatesRef,
    }), [runtime]);
    return React.createElement(Pane, { props });
  }
  const root = createRoot(document.createElement('div'));
  await act(() => root.render(React.createElement(RuntimeProvider, { scope: { backend: 'https://mira.test', account: 'one' } }, React.createElement(Shell))));
  await act(() => {
    runtime.turnCommands.hydrate('a', true, []);
    runtime.updateSessionData('entries', [{ kind: 'msg', msg: { role: 'user', content: 'same' } }, { kind: 'msg', msg: { role: 'assistant', content: 'A private' } }]);
  });
  const before = { shell: shellRenders, composer: globalThis.composerRenders, stream: streamRenders };
  for (let i = 0; i < 100; i++) await act(() => runtime.updateSessionData('entries', previous => appendToken(previous, 'x', i)));
  assert.equal(shellRenders, before.shell); assert.equal(globalThis.composerRenders, before.composer);
  assert.equal(streamRenders, before.stream + 100);
  await act(() => runtime.transition(() => {
    runtime.turnCommands.hydrate('b', true, []);
    runtime.updateSessionData('entries', [{ kind: 'msg', msg: { role: 'user', content: 'same' } }, { kind: 'msg', msg: { role: 'assistant', content: 'B private' } }]);
  }));
  assert.equal(outline.at(-1).msg.content, 'B private');
  await act(() => root.unmount());
});
