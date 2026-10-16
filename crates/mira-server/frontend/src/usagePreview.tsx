// TEMPORARY visual harness for Settings → Usage. Not shipped.
import { createRoot } from 'react-dom/client';
import { applyTheme, watchSystemTheme } from './lib/theme';
import { applyAppearance } from './lib/appearance';
import { UsageSection } from './components/UsageSection';
import './styles.css';

applyTheme();
watchSystemTheme();
applyAppearance();

// Sample usage across three providers, so the page renders without a server.
const days = 30;
const today = new Date();
const since = new Date(today.getTime() - (days - 1) * 86400000).toISOString().slice(0, 10);
const rows: unknown[] = [];
const seeded = (i: number) => (Math.sin(i * 12.9898) * 43758.5453) % 1;
for (let i = 0; i < days; i++) {
  const day = new Date(new Date(`${since}T00:00:00Z`).getTime() + i * 86400000).toISOString().slice(0, 10);
  const wave = 0.6 + 0.4 * Math.sin(i / 3);
  const add = (agent: string | null, model: string, scale: number, sid: string, cost?: number) => {
    const p = Math.round(scale * wave * (0.6 + Math.abs(seeded(i + scale)))); if (p <= 0) return;
    rows.push({ session_id: `${sid}-${i % 5}`, title: `${agent ?? 'Mira'} task ${i % 5}`, cwd: `/Users/me/${i % 2 ? 'mira' : 'faunly-ios'}`, model, day,
      prompt_tokens: p, completion_tokens: Math.round(p * 0.08), cached_input_tokens: Math.round(p * 0.7), agent, cost_usd: cost != null ? cost * wave : null,
      billing: agent ? 'subscription' : 'api' });
  };
  add(null, 'anthropic/claude-sonnet-4.5', 900_000, 'mira');
  add('claude', 'claude-opus-4-1', 2_400_000, 'cc', 6.5);
  if (i > 8) add('codex', 'gpt-5-codex', 1_100_000, 'codex', 2.1);
}
const now = Math.floor(Date.now() / 1000);
const limits = [
  { instance: 'claude-code', driver: 'claude-code', reported_at: now - 420, windows: [
    { name: 'five_hour', utilization: 0.62, resets_at: now + 2 * 3600 + 14 * 60 },
    { name: 'seven_day', utilization: 0.31, resets_at: now + 3 * 86400 },
    { name: 'seven_day_opus', utilization: 0.12, resets_at: now + 3 * 86400 },
  ] },
  { instance: 'codex', driver: 'codex', reported_at: now - 3600 * 5, windows: [
    { name: 'primary', utilization: 0.18, resets_at: now + 3 * 3600 },
    { name: 'seven_day', utilization: 0.74, resets_at: now + 86400 },
  ] },
];
const realFetch = window.fetch.bind(window);
window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
  const url = String(input instanceof Request ? input.url : input);
  const json = (v: unknown) => new Response(JSON.stringify(v), { headers: { 'content-type': 'application/json' } });
  if (url.includes('/api/usage/limits')) return json(limits);
  if (url.includes('/api/usage')) return json({ since, rows });
  return realFetch(input, init);
};

createRoot(document.getElementById('root')!).render(
  <div className="min-h-screen bg-background p-8 text-foreground"><div className="mx-auto max-w-5xl"><UsageSection /></div></div>,
);
