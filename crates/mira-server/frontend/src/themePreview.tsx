// TEMPORARY visual harness: the app without the sign-in gate; not shipped.
import { createRoot } from 'react-dom/client';
import App from './App';
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
applyTheme();
watchSystemTheme();
applyAppearance();
createRoot(document.getElementById('root')!).render(<App />);
