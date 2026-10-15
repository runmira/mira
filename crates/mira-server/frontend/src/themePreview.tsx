// TEMPORARY visual harness: the app without the sign-in gate; not shipped.
import { createRoot } from 'react-dom/client';
import { LazyMotion, domMax } from 'framer-motion';
import App from './App';
import { installWindowDragging } from './lib/desktop';
import { RuntimeProvider } from './state/RuntimeProvider';
import { applyTheme, watchSystemTheme } from './lib/theme';
import { applyAppearance } from './lib/appearance';
import './styles.css';
// ?pretend=0.5.1 makes the server report an old version, to see the
// update-available state against GitHub's real latest release.
const pretend = new URLSearchParams(location.search).get('pretend');
if (pretend) {
  const real = window.fetch.bind(window);
  window.fetch = (input: RequestInfo | URL, init?: RequestInit) =>
    String(input) === '/api/version'
      ? Promise.resolve(new Response(JSON.stringify({ version: pretend }), { headers: { 'content-type': 'application/json' } }))
      : real(input, init);
}
// ?chrome=mac lays the app out as the macOS desktop build does: hidden
// title bar, traffic lights floating top-left (drawn here as dots).
if (new URLSearchParams(location.search).get('chrome') === 'mac') {
  window.__MIRA_DESKTOP__ = true;
  window.__MIRA_CHROME__ = { hiddenTitleBar: true, translucent: false };
  const lights = document.createElement('div');
  lights.style.cssText = 'position:fixed;left:20px;top:24px;display:flex;gap:8px;z-index:100;pointer-events:none';
  for (const c of ['#ff5f57', '#febc2e', '#28c840']) {
    const dot = document.createElement('span');
    dot.style.cssText = `width:12px;height:12px;border-radius:50%;background:${c}`;
    lights.append(dot);
  }
  document.body.append(lights);
}
applyTheme();
watchSystemTheme();
applyAppearance();
installWindowDragging();
// App needs the runtime AuthGate normally provides.
createRoot(document.getElementById('root')!).render(
  <LazyMotion features={domMax}>
    <RuntimeProvider scope={{ backend: window.location.origin, account: 'preview' }}>
      <App />
    </RuntimeProvider>
  </LazyMotion>,
);
