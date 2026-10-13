import assert from 'node:assert/strict';
import { test } from 'node:test';
import React, { act } from 'react';
import { createRoot } from 'react-dom/client';
import { Window } from 'happy-dom';
import { bundle } from './helpers/bundle.mjs';
const window = new Window({ url: 'https://mira.test' });
globalThis.window = window; globalThis.document = window.document; globalThis.IS_REACT_ACT_ENVIRONMENT = true;
let authChanged; const reads = []; let signOuts = 0;
globalThis.authFixture = { auth: {
  getUser: () => new Promise(resolve => reads.push(resolve)),
  signOut: async () => { signOuts++; },
  onAuthStateChange: callback => { authChanged = callback; return { data: { sub: null, subscription: { unsubscribe() {} } } }; },
} };
const { AuthGate } = await bundle(`export { AuthGate } from './src/components/AuthGate';`, {
  'src/lib/supabase': 'export const getSupabase = () => globalThis.authFixture; export const isSupabaseConfigured = () => true;',
  'src/state/RuntimeProvider': 'export function RuntimeProvider({scope, children}) { return <div data-account={scope.account}>{children}</div>; }',
  'src/components/Login': 'export function Login() { return <div>Signed out</div>; }',
});

test('account changes reject stale validation and logout cannot be reversed by an old response', async () => {
  const element = document.createElement('div'); const root = createRoot(element);
  await act(() => root.render(React.createElement(AuthGate, null, React.createElement('p', null, 'Private workspace'))));
  assert.equal(reads.length, 1);
  await act(() => authChanged('SIGNED_IN', { user: { id: 'b' } }));
  assert.equal(reads.length, 2);
  await act(() => reads[1]({ data: { user: { id: 'b' } }, error: null }));
  assert.equal(element.querySelector('[data-account]').dataset.account, 'b');
  await act(() => reads[0]({ data: { user: null }, error: new Error('old token') }));
  assert.equal(signOuts, 0); assert.equal(element.querySelector('[data-account]').dataset.account, 'b');
  await act(() => authChanged('TOKEN_REFRESHED', { user: { id: 'b' } }));
  await act(() => authChanged('SIGNED_OUT', null));
  await act(() => reads[2]({ data: { user: { id: 'b' } }, error: null }));
  assert.equal(element.querySelector('[data-account]'), null);
  await act(() => root.unmount()); await window.happyDOM.abort();
});
