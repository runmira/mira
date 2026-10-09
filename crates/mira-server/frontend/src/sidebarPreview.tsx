/**
 * Dev-only preview of the sidebar with fake chats in every state: running,
 * waiting on you, unread, merged, pinned, forked, launched threads, worktree
 * branches, date sections, a collapsed folder with waiting chats, and the
 * archived list. The API is mocked in this page; nothing reaches a server.
 * Open /sidebar-preview.html on the Vite dev server.
 */
import React, { useState } from 'react';
import ReactDOM from 'react-dom/client';
import './styles.css';
import { Sidebar, type MainView } from './components/Sidebar';
import type { SessionSummary } from './types';

const ROOT = '/Users/damilola/Desktop/coding_agent';
const MIRA = `${ROOT}/mira`;
const LANDING = `${ROOT}/landing`;
const FAUNLY = '/Users/damilola/Desktop/Faunly';
const now = Math.floor(Date.now() / 1000);
const ago = (mins: number) => now - mins * 60;
const DAY = 24 * 60;

function chat(s: Partial<SessionSummary> & { id: string; title: string }): SessionSummary {
  return {
    model: 'claude-sonnet-4-5-20250929',
    cwd: MIRA,
    created_at: ago(DAY * 2),
    updated_at: ago(30),
    message_count: 12,
    active: false,
    ...s,
  };
}

const SESSIONS: SessionSummary[] = [
  chat({ id: 'pinned', title: 'Release checklist for 0.9', pinned: true, updated_at: ago(DAY * 3) }),
  chat({ id: 'active', title: 'Sidebar redesign', updated_at: ago(2), active: true }),
  chat({ id: 'running', title: 'Fix Codex plan mode handoff', agent_driver: 'codex', model: 'gpt-5-codex', updated_at: ago(1) }),
  chat({
    id: 'waiting', title: 'Add Stripe webhooks', agent_driver: 'claude-code', updated_at: ago(6),
    needs_attention: true, worktree_branch: 'feature/stripe-webhooks', worktree_status: 'unmerged',
    pr: { number: 431, title: 'Stripe webhooks', url: 'https://github.com/example/mira/pull/431', state: 'open' },
  }),
  chat({
    id: 'thread-1', title: 'Webhook retries + backoff', agent_driver: 'codex', model: 'gpt-5-codex', updated_at: ago(4),
    launched_by: 'waiting', worktree_branch: 'feature/webhook-retries', worktree_status: 'unmerged',
    pr: { number: 433, title: 'Webhook retries', url: 'https://github.com/example/mira/pull/433', state: 'draft' },
  }),
  chat({ id: 'thread-2', title: 'Docs for the webhook endpoint', launched_by: 'waiting', updated_at: ago(20), needs_attention: true }),
  chat({ id: 'unread', title: 'Audit MCP elicitation replies', agent_driver: 'codex', model: 'gpt-5-codex', updated_at: ago(45) }),
  chat({ id: 'fork', title: 'Sidebar redesign (fork)', forked_from: 'active', forked_at: 'Make the rows denser', updated_at: ago(15) }),
  chat({
    id: 'merged', title: 'Remove dead delegate code', updated_at: ago(DAY + 120),
    worktree_branch: 'chore/dead-delegate', worktree_status: 'merged',
  }),
  chat({
    id: 'pr-merged', title: 'Usage ring cache share', updated_at: ago(DAY + 300),
    worktree_branch: 'fix/token-usage', worktree_status: 'unmerged',
    pr: { number: 175, title: 'Token usage', url: 'https://github.com/example/mira/pull/175', state: 'merged', closed_at: ago(DAY) },
  }),
  chat({
    id: 'pr-closed', title: 'Try a denser composer', updated_at: ago(DAY * 3),
    worktree_branch: 'exp/composer', worktree_status: 'unmerged',
    pr: { number: 160, title: 'Denser composer', url: 'https://github.com/example/mira/pull/160', state: 'closed', closed_at: ago(DAY * 2) },
  }),
  chat({
    id: 'pr-reopened', title: 'Evals follow-up', updated_at: ago(10),
    worktree_branch: 'evals/more', worktree_status: 'unmerged',
    pr: { number: 176, title: 'Evals', url: 'https://github.com/example/mira/pull/176', state: 'merged', closed_at: ago(DAY) },
  }),
  chat({ id: 'settled-by-hand', title: 'Answer the pricing question', updated_at: ago(DAY * 2), settled_at: ago(DAY) }),
  chat({ id: 'week', title: 'Investigate the flaky worktree test', updated_at: ago(DAY * 4), model: 'gpt-5' }),
  chat({ id: 'older', title: 'Initial project setup', updated_at: ago(DAY * 40) }),
  chat({ id: 'older-2', title: 'Try the ACP adapter for Grok', agent_driver: 'grok', updated_at: ago(DAY * 70) }),
  chat({ id: 'landing-1', title: 'Hero copy pass', cwd: LANDING, updated_at: ago(90), needs_attention: true, agent_driver: 'claude-code' }),
  chat({ id: 'landing-2', title: 'Pricing page layout', cwd: LANDING, updated_at: ago(DAY * 2) }),
  chat({ id: 'faunly-1', title: 'Onboarding flow bugs', cwd: FAUNLY, updated_at: ago(DAY * 9), agent_driver: 'opencode' }),
  chat({ id: 'faunly-2', title: '', first_user_message: 'Why does the map crash on Android?', cwd: FAUNLY, updated_at: ago(DAY * 12) }),
];

const ARCHIVED: SessionSummary[] = [
  chat({ id: 'arch-1', title: 'Old auth experiment', archived: true, updated_at: ago(DAY * 20) }),
  chat({ id: 'arch-2', title: 'Spike: Tauri v2 migration', archived: true, agent_driver: 'claude-code', updated_at: ago(DAY * 95) }),
];

const PREVIEWS: Record<string, object> = {
  waiting: {
    last_user: 'Add a Stripe webhook endpoint that verifies signatures and records payments.',
    last_assistant: "I need the webhook signing secret to test this. I asked for it with request_secret — it's waiting in the composer.",
    last_tool: 'request_secret',
    message_count: 9,
  },
  running: {
    last_user: 'Plan mode should hand off cleanly when I approve the plan.',
    last_assistant: 'Tracing the turn end in appserver.rs — the plan item arrives before turn/completed, so I hold the end until review.',
    last_tool: 'Read appserver.rs',
    message_count: 14,
  },
};

// The page's API: just what the sidebar and its hover card ask for.
const realFetch = window.fetch.bind(window);
window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
  const url = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url, location.href);
  const json = (v: unknown) => new Response(JSON.stringify(v), { headers: { 'content-type': 'application/json' } });
  if (url.pathname === '/api/sessions') {
    return json(url.searchParams.get('archived') === 'true' ? ARCHIVED : SESSIONS);
  }
  const preview = /^\/api\/sessions\/([^/]+)\/preview$/.exec(url.pathname);
  if (preview) {
    const id = decodeURIComponent(preview[1]);
    return json(PREVIEWS[id] ?? {
      last_user: 'Can you take a look at this?',
      last_assistant: 'Done — the change is in place and the tests pass.',
      last_tool: 'bash',
      message_count: 6,
    });
  }
  if (url.pathname.startsWith('/api/')) return json({});
  return realFetch(input, init);
};

// Seed the sidebar's own remembered state: one unread chat, one collapsed
// folder (to show its waiting count).
try {
  localStorage.setItem('mira.sidebar.unread', JSON.stringify(['unread']));
  localStorage.setItem('mira.sidebar.collapsed-projects', JSON.stringify([LANDING]));
} catch { /* private mode */ }

function Preview() {
  const [view, setView] = useState<MainView>('chat');
  const [active, setActive] = useState('active');
  return (
    <main className="flex h-screen bg-background text-foreground">
      <div className="h-full w-[290px] shrink-0 border-r border-border bg-panel">
        <Sidebar
          status="open"
          cwd={MIRA}
          activeSessionId={active}
          activeBusy={false}
          runningSessions={new Set(['running', 'thread-1', 'waiting', 'thread-2', 'landing-1'])}
          completedSessions={new Map()}
          refreshKey={0}
          activeView={view}
          onNavigate={setView}
          onNewChat={() => {}}
          onOpenSettings={() => {}}
          onOpenPicker={() => {}}
          onSessionLoaded={() => {}}
          onAttachSession={setActive}
          activePr={{ number: 412, title: 'Sidebar: needs-you badges, search, threads' }}
        />
      </div>
      <section className="flex-1 overflow-auto p-8 text-[13px] text-muted-foreground">
        <h1 className="mb-3 text-[15px] font-semibold text-foreground">Sidebar states (fake data)</h1>
        <ul className="list-disc space-y-1 pl-5">
          <li><b>Add Stripe webhooks</b>: waiting on you (amber), worktree branch, two launched threads nested under it</li>
          <li><b>Webhook retries + backoff</b>: launched thread, running; <b>Docs for the webhook endpoint</b>: launched thread, waiting</li>
          <li><b>Fix Codex plan mode handoff</b>: running</li>
          <li><b>Audit MCP elicitation replies</b>: finished while you were away (unread dot)</li>
          <li><b>Sidebar redesign</b>: the open chat, with a fork under it</li>
          <li><b>Recents</b>: the five latest chats across folders</li>
          <li><b>Settled</b> (bottom, collapsed): PR merged, PR closed, branch merged, settled by hand</li>
          <li><b>Evals follow-up</b>: PR merged but you wrote to it since, so it stays in its folder</li>
          <li>PR marks: open and draft in grey, merged in colour; forks use the split mark</li>
          <li><b>Release checklist</b>: pinned · date sections · “Show more” past 5 chats</li>
          <li><b>landing</b>: collapsed folder showing its waiting count</li>
          <li>Hover any row for the peek; type in Search; open Archived at the bottom</li>
        </ul>
        <p className="mt-4">Theme: <button className="underline" onClick={() => document.documentElement.classList.toggle('dark')}>toggle dark class</button></p>
      </section>
    </main>
  );
}

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <Preview />
  </React.StrictMode>,
);
