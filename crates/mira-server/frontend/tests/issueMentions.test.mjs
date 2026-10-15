import assert from 'node:assert/strict';
import { test } from 'node:test';
import React, { act } from 'react';
import { createRoot } from 'react-dom/client';
import { Window } from 'happy-dom';
import { bundle } from './helpers/bundle.mjs';

const { issueTrigger, splitIssueTokens, rememberIssue, MentionInput, useIssuePicker, QueryClient, QueryClientProvider } = await bundle(`
export { issueTrigger, splitIssueTokens, rememberIssue } from './src/lib/issueMentions';
export { MentionInput } from './src/components/MentionInput';
export { useIssuePicker } from './src/components/composer/IssuePalette';
export { QueryClient, QueryClientProvider } from '@tanstack/react-query';`, {
  'src/state/RuntimeProvider': `export function useAppRuntime(){return {resourceKey:(r,...id)=>['mira',r,...id]};}`,
});

const window = new Window();
globalThis.window = window;
globalThis.document = window.document;
globalThis.Node = window.Node;
globalThis.HTMLElement = window.HTMLElement;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

test('a trailing # opens the picker, but not inside words or after a space', () => {
  assert.deepEqual(issueTrigger('#'), { query: '', triggerStart: 0 });
  assert.deepEqual(issueTrigger('fixes #20'), { query: '20', triggerStart: 6 });
  assert.deepEqual(issueTrigger('see #handoff'), { query: 'handoff', triggerStart: 4 });
  assert.equal(issueTrigger('C#'), null);
  assert.equal(issueTrigger('# heading'), null);
  assert.equal(issueTrigger('runmira/mira#205 '), null);
});

test('references split out of prose whole', () => {
  assert.deepEqual(splitIssueTokens('see runmira/mira#205, then (a/b#3)'), [
    { kind: 'text', text: 'see ' },
    { kind: 'issue', token: 'runmira/mira#205', number: 205 },
    { kind: 'text', text: ', then (' },
    { kind: 'issue', token: 'a/b#3', number: 3 },
    { kind: 'text', text: ')' },
  ]);
  assert.deepEqual(splitIssueTokens('C# and #5'), [{ kind: 'text', text: 'C# and #5' }]);
});

test('a picked issue becomes a chip that serializes back to its reference', async () => {
  rememberIssue('runmira/mira#205', { kind: 'issue', number: 205, title: 'Context handoff', state: 'open', author: null, labels: [], updated_at: null });
  const host = document.createElement('div');
  document.body.append(host);
  const root = createRoot(host);
  const handle = { current: null };
  let text = '';
  await act(async () => {
    root.render(React.createElement(MentionInput, { value: '', onChange: (t) => { text = t; }, placeholder: '', roster: [], handleRef: handle }));
  });
  await act(async () => handle.current.setText('please do runmira/mira#205 next'));
  const chip = host.querySelector('[data-issue]');
  assert.ok(chip, 'rendered as a chip');
  assert.equal(chip.getAttribute('contenteditable'), 'false');
  assert.match(chip.textContent, /#205\s*Context handoff/);
  assert.equal(text, 'please do runmira/mira#205 next');
  await act(async () => root.unmount());
});

test('picking replaces the #query with the reference and Escape keeps the text', async () => {
  const original = globalThis.fetch;
  globalThis.fetch = async (_url, options) => {
    requests += 1;
    assert.equal(new Headers(options.headers).get('X-Mira-Session'), 's1');
    return new Response(
      JSON.stringify({
        owner: 'runmira',
        repo: 'mira',
        error: null,
        items: [{ kind: 'pr', number: 207, title: 'Compaction', state: 'merged', author: 'dami', labels: [], updated_at: null }],
      }),
      { status: 200, headers: { 'Content-Type': 'application/json' } },
    );
  };
  let picker;
  let written = null;
  let requests = 0;
  const client = new QueryClient();
  function Probe({ text }) {
    picker = useIssuePicker({ text, enabled: true, sessionId: 's1', updateText: (t) => { written = t; } });
    return null;
  }
  const host = document.createElement('div');
  const root = createRoot(host);
  try {
    const render = (text) => root.render(React.createElement(QueryClientProvider, { client }, React.createElement(Probe, { text })));
    // Fast typing settles into one request.
    await act(async () => render('ship #2'));
    await act(async () => render('ship #20'));
    for (let i = 0; i < 40 && !picker.result; i++) {
      await act(async () => new Promise((r) => setTimeout(r, 25)));
    }
    assert.equal(picker.open, true);
    assert.equal(picker.result.items[0].number, 207);
    assert.equal(requests, 1, 'debounced');
    const key = (k) => ({ key: k, shiftKey: false, metaKey: false, ctrlKey: false, preventDefault() {} });
    await act(async () => { assert.equal(picker.onKeyDown(key('Enter')), true); });
    assert.equal(written, 'ship runmira/mira#207 ');
    await act(async () => { picker.onKeyDown(key('Escape')); });
    assert.equal(picker.open, false, 'dismissed');
  } finally {
    globalThis.fetch = original;
    await act(async () => root.unmount());
  }
});
