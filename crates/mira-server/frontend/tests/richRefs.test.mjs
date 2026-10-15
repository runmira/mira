import assert from 'node:assert/strict';
import { test } from 'node:test';
import React, { act } from 'react';
import { createRoot } from 'react-dom/client';
import { Window } from 'happy-dom';
import { bundle } from './helpers/bundle.mjs';
const { CommitChip, setScope } = await bundle(`
export { CommitChip } from './src/components/RichRefs';
export { setScope } from './src/hooks/useReferenceScope';`, {
  'src/hooks/useReferenceScope': `let scope={cacheKey:'a',session:'chat-a'}; export function setScope(next){scope=next;} export function useReferenceScope(){return scope;}`,
  'src/components/ui/hover-card': `export function HoverCard({trigger,children,onOpen}){return <div>{trigger}<button onClick={onOpen}>Hover</button>{children}</div>}`,
});
const window = new Window();
globalThis.window = window; globalThis.document = window.document; globalThis.IS_REACT_ACT_ENVIRONMENT = true;
const card = { sha:'85a6218abcdef',short:'85a6218',subject:'Fix compaction',body:'',author:'Test',date:1,files:[] };
const response = status => new Response(status === 200 ? JSON.stringify(card) : '{}', { status, headers:{'Content-Type':'application/json'} });

test('commit details use their chat owner, retry misses, and reject stale completions', async () => {
  const original = globalThis.fetch;
  const calls = [];
  let pendingA;
  let attempts = 0;
  globalThis.fetch = (url, options) => {
    const session = new Headers(options.headers).get('X-Mira-Session'); calls.push({url,session});
    if (session === 'chat-a') return new Promise(resolve => {pendingA=resolve;});
    return Promise.resolve(response(++attempts === 1 ? 404 : 200));
  };
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
  try {
    setScope({cacheKey:'a',session:'chat-a'});
    await act(()=>root.render(React.createElement(CommitChip,{sha:'85a6218'})));
    await act(()=>host.querySelector('button').click());
    setScope({cacheKey:'b',session:'chat-b'});
    await act(()=>root.render(React.createElement(CommitChip,{sha:'85a6218'})));
    await act(()=>host.querySelector('button').click());
    assert.match(host.textContent,/Not a commit/);
    await act(()=>host.querySelector('button').click());
    assert.match(host.textContent,/Fix compaction/,'a failed lookup must not remain cached forever');
    await act(()=>pendingA(response(404)));
    assert.match(host.textContent,/Fix compaction/,'an old chat completion must not overwrite the new chat');
    assert.deepEqual(calls.map(call=>call.session),['chat-a','chat-b','chat-b']);
  } finally { await act(()=>root.unmount());host.remove();globalThis.fetch=original; }
});

test('network and auth failures are retryable errors, not a false missing-commit verdict', async () => {
  const original=globalThis.fetch;let attempts=0;
  globalThis.fetch=async()=>response(++attempts===1?401:200);
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
  try {
    setScope({cacheKey:'errors',session:'chat-errors'});
    await act(()=>root.render(React.createElement(CommitChip,{sha:'85a6218'})));
    await act(()=>host.querySelector('button').click());
    assert.match(host.textContent,/unavailable/);assert.doesNotMatch(host.textContent,/Not a commit/);
    await act(()=>host.querySelector('button').click());
    assert.match(host.textContent,/Fix compaction/);
  } finally { await act(()=>root.unmount());host.remove();globalThis.fetch=original; }
});
