/**
 * Dev-only preview of session mentions in the chat stream: bare
 * `sess_…` ids render as chips with hover cards, the way links and
 * PR/issue references do. Open /session-preview.html on the Vite dev
 * server.
 */
import React from 'react';
import ReactDOM from 'react-dom/client';
import './styles.css';
import { Markdown } from './components/Markdown';
import { UserRichText } from './components/UserRichText';
import { applyTheme } from './lib/theme';
import { installInputModality } from './lib/inputModality';

applyTheme();
installInputModality();

const A = 'sess_a41504d51dd44bfca7e51968e0e88f24';
const B = 'sess_cbe42bdbc3d442c9814188145e99af38';
const C = 'sess_0123456789abcdef0123456789abcdef';
const UNKNOWN = 'sess_ffffffffffffffffffffffffffffffff';

const PREVIEWS: Record<string, unknown> = {
  [A]: {
    id: A,
    title: 'Stripe webhooks retry',
    last_user: 'Can you add retries to the webhook handler?',
    last_assistant: 'Done — exponential backoff with three retries, tests pass.',
    last_tool: 'bash',
    message_count: 14,
    updated_at: Math.floor(Date.now() / 1000) - 3600,
  },
  [B]: {
    id: B,
    title: null,
    last_user: 'What did we decide about the queue?',
    last_assistant: null,
    last_tool: null,
    message_count: 3,
    updated_at: Math.floor(Date.now() / 1000) - 86400,
  },
  [C]: {
    id: C,
    title: 'A very long session title that should truncate inside the inline chip instead of stretching the line',
    last_user: 'Keep going with the migration.',
    last_assistant: 'Migrated three more modules.',
    last_tool: null,
    message_count: 22,
    updated_at: Math.floor(Date.now() / 1000) - 600,
  },
};

// The page's API: just what the session chip asks for. Everything else
// falls through to the dev proxy (or fails quietly offline).
const realFetch = window.fetch.bind(window);
window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
  const url = new URL(
    typeof input === 'string' ? input : input instanceof URL ? input.href : input.url,
    location.href,
  );
  const preview = /^\/api\/sessions\/([^/]+)\/preview$/.exec(url.pathname);
  if (preview) {
    const id = decodeURIComponent(preview[1]);
    const found = PREVIEWS[id];
    if (!found) return new Response('load: not found', { status: 404 });
    return new Response(JSON.stringify(found), { headers: { 'content-type': 'application/json' } });
  }
  if (url.pathname === '/api/sessions') {
    const now = Math.floor(Date.now() / 1000);
    return new Response(
      JSON.stringify([
        {
          id: A, model: 'gpt-6-astra', cwd: '/Users/damilola/Desktop/coding_agent/mira',
          created_at: now - 7200, updated_at: now - 3600, message_count: 14,
          title: 'Stripe webhooks retry', first_user_message: 'Can you add retries?',
          active: false, running: false,
        },
        {
          id: B, model: 'claude-opus-4-6', cwd: '/Users/damilola/Desktop/coding_agent/landing',
          created_at: now - 90000, updated_at: now - 86400, message_count: 3,
          title: null, first_user_message: 'What did we decide about the queue?',
          active: false, running: false,
        },
        {
          id: C, model: 'gpt-6-astra', cwd: '/Users/damilola/Desktop/coding_agent/mira',
          created_at: now - 1200, updated_at: now - 600, message_count: 22,
          title: 'A very long session title that should truncate inside the inline chip instead of stretching the line',
          first_user_message: 'Keep going with the migration.',
          active: false, running: true, worktree_branch: 'feat/queue-migration',
          worktree_status: 'unmerged',
          pr: { number: 431, title: 'Queue migration', url: 'https://github.com/example/mira/pull/431', state: 'open' },
        },
      ]),
      { headers: { 'content-type': 'application/json' } },
    );
  }
  return realFetch(input, init);
};

const ASSISTANT_SAMPLES = [
  `I moved that work to ${A} — the webhook retry is in place there.`,
  `Comparing ${A} and ${B}: the first has the fix, the second is still Untitled.`,
  `Trailing punctuation stays outside the chip: see ${A}. Next sentence.`,
  `Unknown sessions explain themselves: ${UNKNOWN} shows a not-found card.`,
  `Code is off-limits: \`${A}\` stays plain, and so does a fence:\n\`\`\`\n${B}\n\`\`\``,
  `Mixed with the existing chips: example/mira#431 and https://example.com/docs sit next to ${A}.`,
  `Long titles truncate instead of stretching the line: ${C} keeps flowing.`,
];

const USER_SAMPLES = [
  `please continue in ${A}`,
  `what is the status of ${B}?`,
];

function Preview() {
  const [opened, setOpened] = React.useState<string | null>(null);
  // The chip dispatches `mira:open-session`; the real app attaches the
  // session on it (see `useWorkspaceEffects`). Here there is no
  // workspace, so surface the click instead of swallowing it.
  React.useEffect(() => {
    const onOpen = (e: Event) => setOpened((e as CustomEvent<string>).detail);
    window.addEventListener('mira:open-session', onOpen);
    return () => window.removeEventListener('mira:open-session', onOpen);
  }, []);
  return (
    <main className="min-h-screen bg-background text-foreground">
      <div className="mx-auto flex max-w-2xl flex-col gap-8 px-8 py-10">
        <div>
          <h1 className="text-lg font-semibold">Session mentions</h1>
          <p className="mt-1 text-[13px] text-muted-foreground">
            Bare <code className="md-inline">sess_…</code> ids chip like PRs and links. Hover for the
            card, click opens the session.
          </p>
          {opened && (
            <div className="mt-2 rounded-lg border border-mira-blue/30 bg-mira-blue/[0.08] px-3 py-2 font-mono text-[12px] text-mira-blue">
              mira:open-session → {opened}
            </div>
          )}
        </div>
        <section className="flex flex-col gap-5">
          <h2 className="text-[12px] font-medium uppercase tracking-wide text-muted-foreground">
            Assistant stream (Markdown)
          </h2>
          {ASSISTANT_SAMPLES.map((text, i) => (
            <div key={i} data-assistant-message className="text-[14px] leading-relaxed">
              <Markdown text={text} />
            </div>
          ))}
        </section>
        <section className="flex flex-col gap-3">
          <h2 className="text-[12px] font-medium uppercase tracking-wide text-muted-foreground">
            Sent messages (UserRichText)
          </h2>
          {USER_SAMPLES.map((text, i) => (
            <div
              key={i}
              className="self-end rounded-2xl bg-fg/[0.07] px-3 py-2 text-[14px] leading-relaxed"
            >
              <UserRichText text={text} roster={[]} />
            </div>
          ))}
        </section>
      </div>
    </main>
  );
}

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <Preview />
  </React.StrictMode>,
);
