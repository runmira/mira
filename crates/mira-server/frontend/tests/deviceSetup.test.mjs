import assert from 'node:assert/strict';
import { after, test } from 'node:test';
import { Window } from 'happy-dom';
import { bundle } from './helpers/bundle.mjs';

const browser = new Window({ url: 'http://localhost/' });
for (const key of ['window','document','navigator','localStorage','HTMLElement','HTMLInputElement','Element','Node','Event','CustomEvent','MutationObserver','NodeFilter','ResizeObserver','DocumentFragment','ShadowRoot','CSS','SVGElement','HTMLButtonElement']) Object.defineProperty(globalThis, key, { configurable: true, value: browser[key] });
globalThis.getComputedStyle = browser.getComputedStyle.bind(browser);
globalThis.requestAnimationFrame = browser.requestAnimationFrame.bind(browser);
globalThis.cancelAnimationFrame = browser.cancelAnimationFrame.bind(browser);
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
document.write('<!doctype html><html><body></body></html>');
after(() => browser.happyDOM.abort());

const { act } = await import('react');
const { createRoot } = await import('react-dom/client');
const { render, client } = await bundle(`
  import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
  import { DevicePane } from './src/components/device/DevicePane';
  // No garbage-collection timers, so Node can exit when the test ends.
  export const client = new QueryClient({ defaultOptions: { queries: { gcTime: Infinity }, mutations: { gcTime: Infinity } } });
  export const render = () => <QueryClientProvider client={client}><DevicePane /></QueryClientProvider>;
`);

const off = {
  hub: 'off', hubDetail: null, hubProgress: null, hubEnabled: false, agentAccess: false, agentTools: 'absent', agentDetail: null, agentProgress: null,
  platforms: { ios: { available: true, reason: null }, android: { available: false, reason: 'Android SDK not found.' } },
  versions: { hub: '0.15.3', agentDevice: '0.21.25', node: null, hubInstalled: false, agentDeviceInstalled: false },
};
const json = (body) => new Response(JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' } });
const settle = () => act(async () => { await new Promise((r) => setTimeout(r, 15)); });

test('setup downloads nothing until the hub switch is turned on', async () => {
  const calls = [];
  globalThis.fetch = async (url, init = {}) => {
    calls.push([url, init.method ?? 'GET', init.body ?? null]);
    if (url === '/api/devices/state') return json(off);
    if (url === '/api/devices/settings') return json({ ...off, hubEnabled: true, hub: 'installing', hubDetail: 'Installing device hub…', hubProgress: 0.42 });
    throw new Error(`unexpected ${url}`);
  };
  const host = document.createElement('div');
  document.body.append(host);
  const root = createRoot(host);
  await act(async () => root.render(render()));
  await settle();

  assert.match(host.textContent, /Bring devices into Mira/);
  assert.deepEqual(calls.filter(([, m]) => m !== 'GET'), [], 'opening the pane changes nothing');

  const toggle = host.querySelector('[role="switch"]');
  await act(async () => toggle.click());
  await settle();
  assert.deepEqual(calls.filter(([, m]) => m === 'POST').map(([u, , b]) => [u, JSON.parse(b)]), [['/api/devices/settings', { hubEnabled: true }]]);
  // The mutation's answer lands in the shared cache: no refetch needed.
  assert.match(host.textContent, /Installing device hub…/);
  const bar = host.querySelector('[role="progressbar"]');
  assert.equal(bar.getAttribute('aria-valuenow'), '42', 'the server’s progress drives the bar');

  await act(async () => root.unmount());
  client.clear();
  host.remove();
});
