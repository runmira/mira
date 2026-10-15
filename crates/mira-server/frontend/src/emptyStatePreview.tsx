/**
 * Dev-only preview of the new-chat empty state and its project switcher
 * (click the dotted project name, or the composer's folder chip). The
 * sessions list is faked here; nothing reaches a server.
 * Open /empty-state-preview.html on the Vite dev server.
 */
import { LazyMotion, domMax } from 'framer-motion';
import React, { useState } from 'react';
import ReactDOM from 'react-dom/client';
import './styles.css';
import { ProjectChip } from './components/composer/ComposerMenus';
import { EmptyState } from './components/transcript/EmptyState';
import { RuntimeProvider } from './state/RuntimeProvider';
import type { SessionSummary } from './types';

const ROOT = '/Users/damilola/Desktop/coding_agent';
const now = Math.floor(Date.now() / 1000);
const FOLDERS: [string, number, number][] = [
  [`${ROOT}/mira`, 14, 2],
  [`${ROOT}/landing`, 3, 90],
  ['/Users/damilola/Desktop/Faunly', 6, 60 * 24 * 3],
  [`${ROOT}/mira-service`, 1, 60 * 24 * 12],
  ['/Users/damilola/Desktop/showreel', 2, 60 * 24 * 40],
];
const SESSIONS: SessionSummary[] = FOLDERS.flatMap(([cwd, count, mins]) =>
  Array.from({ length: count }, (_, i) => ({
    id: `${cwd}-${i}`,
    title: i === 0 ? 'Sidebar redesign' : `Chat ${i}`,
    cwd,
    model: 'claude-sonnet-4-5',
    created_at: now - 86400 * 30,
    updated_at: now - mins * 60 - i * 3600,
    message_count: 4,
    active: false,
  })),
);
const realFetch = window.fetch.bind(window);
window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
  const url = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url, window.location.origin);
  if (url.pathname === '/api/sessions') return new Response(JSON.stringify(SESSIONS), { headers: { 'content-type': 'application/json' } });
  if (url.pathname.startsWith('/api/')) return new Response('{}', { headers: { 'content-type': 'application/json' } });
  return realFetch(input, init);
};

function Preview() {
  const [cwd, setCwd] = useState(`${ROOT}/mira`);
  const [note, setNote] = useState('');
  const toggleTheme = () => {
    const light = document.documentElement.dataset.theme !== 'light';
    document.documentElement.dataset.theme = light ? 'light' : 'dark';
    document.documentElement.classList.toggle('dark', !light);
  };
  return (
    <main className="flex h-screen flex-col bg-background text-foreground">
      <div className="flex items-center gap-3 border-b border-border/60 px-4 py-2 text-[12px] text-muted-foreground">
        <button className="underline" onClick={toggleTheme}>Toggle light / dark</button>
        {note && <span>{note}</span>}
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        <EmptyState
          cwd={cwd}
          onPrompt={() => {}}
          onOpenSession={() => {}}
          onSwitchProject={(path) => {
            setCwd(path);
            setNote(`Switched to ${path}`);
          }}
          onNewProject={() => setNote('Open folder… (the folder panel)')}
        />
      </div>
      <div className="mx-auto mb-6 flex w-full max-w-3xl items-center rounded-2xl border border-border/70 px-3 py-2">
        <span className="flex-1 text-[13px] text-muted-foreground/60">Ask Mira anything</span>
        <ProjectChip cwd={cwd} onSwitchProject={setCwd} onNewProject={() => setNote('Open folder…')} />
      </div>
    </main>
  );
}

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <LazyMotion features={domMax}>
      <RuntimeProvider scope={{ backend: window.location.origin, account: 'preview' }}><Preview /></RuntimeProvider>
    </LazyMotion>
  </React.StrictMode>,
);
