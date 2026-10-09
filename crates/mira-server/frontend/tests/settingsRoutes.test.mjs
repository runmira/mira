import assert from 'node:assert/strict';
import { after, test } from 'node:test';
import { Window } from 'happy-dom';
import { bundle } from './helpers/bundle.mjs';

const window = new Window({ url: 'http://localhost/' });
for (const key of [
  'window',
  'document',
  'navigator',
  'localStorage',
  'HTMLElement',
  'HTMLInputElement',
  'Element',
  'Node',
  'Event',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'customElements',
  'CSSStyleSheet',
  'DocumentFragment',
  'ShadowRoot',
  'CSS',
  'SVGElement',
  'requestAnimationFrame',
  'cancelAnimationFrame',
]) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value:
      typeof window[key] === 'function' && /AnimationFrame/.test(key)
        ? window[key].bind(window)
        : window[key],
  });
}
document.write('<!doctype html><html><body></body></html>');
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
const { act } = await import('react');
const fixture = {
  default_provider: 'groq',
  default_model: 'original-model',
  default_mode: 'manual',
  providers: [{ name: 'groq', base_url: 'https://api.groq.com/openai/v1', has_api_key: true }],
  keys: [],
  configured: true,
  config_path: 'test.yaml',
  memory: { auto_extract: true, tools_enabled: true, inject_context: true, extractor_model: null },
};
let settingsReads = 0;
globalThis.fetch = async (url) => {
  if (String(url) === '/api/settings') {
    settingsReads++;
    return new Response(JSON.stringify(fixture));
  }
  return new Response('{}');
};
const { mount } = await bundle(
  `
import { createElement } from 'react';
import { createRoot } from 'react-dom/client';
import { createMemoryRouter, RouterProvider, Outlet } from 'react-router';
import { settingsRoute } from './src/app/settingsRoutes';
import { useAppNavigation } from './src/app/useAppNavigation';
import { SettingsNavigationContext } from './src/components/settings/SettingsContext';
export function mount(element, initial) {
  let navigation;
  function Root() {
    navigation = useAppNavigation();
    return <SettingsNavigationContext.Provider value={{ section: navigation.settingsSection, onSectionChange: navigation.setSettingsSection, onSaved: () => {}, onExit: navigation.exitSettings }}><Outlet /></SettingsNavigationContext.Provider>;
  }
  const router = createMemoryRouter([{ path: '/', Component: Root, HydrateFallback: () => null, children: [settingsRoute, {path: 'chat', element: <p>Chat</p>}, {path: 'plugins', element: <p>Plugins</p>}]}], { initialEntries: [initial] });
  const root = createRoot(element);
  root.render(<RouterProvider router={router} />);
  return { router, root, navigation: () => navigation };
}
`,
  { 'src/components/diffs/StyledDiffCodeView': 'export const StyledDiffCodeView = () => null;' },
);

async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 30));
  });
}
let mounted;
after(async () => {
  if (mounted) {
    await act(async () => mounted.root.unmount());
    mounted.router.dispose();
  }
  await window.happyDOM.abort();
});

test('settings deep links and history preserve a draft without refetching', async () => {
  const element = document.createElement('div');
  document.body.append(element);
  await act(async () => {
    mounted = mount(element, '/settings/memory');
  });
  await settle();
  assert.equal(mounted.router.state.location.pathname, '/settings/memory');
  assert.match(element.textContent, /Cross-session memory/);
  const input = element.querySelector('input');
  assert.ok(input, 'memory extractor input is rendered');
  // Use the native setter so React sees an actual input event.
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(
      input,
      'draft-extractor',
    );
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  assert.match(element.textContent, /Unsaved changes/);
  await act(async () => mounted.router.navigate('/settings/provider'));
  await settle();
  assert.match(element.textContent, /Model provider/);
  await act(async () => mounted.router.navigate(-1));
  await settle();
  assert.equal(element.querySelector('input').value, 'draft-extractor');
  assert.equal(settingsReads, 1, 'section navigation does not reload or rehydrate the draft');
  await act(async () => mounted.router.navigate(1));
  await settle();
  assert.equal(mounted.router.state.location.pathname, '/settings/provider');
  await act(async () => mounted.root.unmount());
  mounted.router.dispose();
  mounted = null;
});

test('opening settings remembers the previous page and two same-event commands compose', async () => {
  const element = document.createElement('div');
  document.body.append(element);
  await act(async () => {
    mounted = mount(element, '/plugins');
  });
  await settle();
  await act(async () => {
    mounted.navigation().openSettings();
    mounted.navigation().setSettingsSection('memory');
  });
  await settle();
  assert.equal(mounted.router.state.location.pathname, '/settings/memory');
  await act(async () => mounted.navigation().exitSettings());
  await settle();
  assert.equal(mounted.router.state.location.pathname, '/plugins');
});
