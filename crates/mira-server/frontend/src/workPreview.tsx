import { RuntimeProvider } from './state/RuntimeProvider';
/**
 * Dev-only prototype of the sidebar's Work tab. Chat is organised by
 * conversation; Work by deliverable: an issue / branch / PR, with every
 * chat, fork, launched thread and agent that touched it. This page runs the
 * real Work containers and queries against a fake API (sessions, PR detail
 * and files, branch, diff, issue search, worktree, merge). Nothing reaches a
 * server.
 * Open /work-preview.html on the Vite dev server.
 */
import { LazyMotion, domMax } from 'framer-motion';
import React, { useState } from 'react';
import ReactDOM from 'react-dom/client';
import './styles.css';
import { createMemoryRouter, RouterProvider, useLocation, useNavigate } from 'react-router';
import type { CheckRunView, PullRequestDetailView, ReviewView } from './api/pullRequests';
import { Sidebar, type MainView } from './components/Sidebar';
import { WorkView } from './components/work/WorkView';
import type { IssueOption } from './lib/issueMentions';
import type { SessionSummary } from './types';

const ROOT = '/Users/damilola/Desktop/coding_agent';
const MIRA = `${ROOT}/mira`;
const LANDING = `${ROOT}/landing`;
const WT = `${MIRA}/.mira/worktrees`;
const now = Math.floor(Date.now() / 1000);
const ago = (mins: number) => now - mins * 60;
const DAY = 24 * 60;
const prOf = (number: number, title: string, state: 'open' | 'draft' | 'merged' | 'closed' = 'open', closed?: number) =>
  ({ number, title, url: `https://github.com/runmira/mira/pull/${number}`, state, closed_at: closed ?? null });

function chat(s: Partial<SessionSummary> & { id: string; title: string }): SessionSummary {
  return { model: 'claude-sonnet-4-5-20250929', cwd: MIRA, created_at: ago(DAY), updated_at: ago(30), message_count: 12, active: false, ...s };
}

const SESSIONS: SessionSummary[] = [
  // Needs you: an approval on the lead chat, a thread still running.
  chat({ id: 'stripe', title: 'Add Stripe webhooks', agent_driver: 'claude-code', created_at: ago(300), updated_at: ago(6), cwd: `${WT}/issue-212-stripe-webhooks`,
    first_user_message: 'Payments made through Checkout never reach our database. Add a webhook endpoint that verifies Stripe signatures and records checkout.session.completed.',
    needs_attention: true, attention_reason: 'Approve 3 commands', running: true, worktree_branch: 'issue-212-stripe-webhooks', worktree_status: 'unmerged' }),
  chat({ id: 'stripe-docs', title: 'Docs for the webhook endpoint', launched_by: 'stripe', created_at: ago(120), updated_at: ago(12), running: true }),
  chat({ id: 'stripe-fork', title: 'Add Stripe webhooks (fork)', forked_from: 'stripe', forked_at: 'Try idempotency keys instead', agent_driver: 'codex', model: 'gpt-5-codex', created_at: ago(90), updated_at: ago(40) }),
  // Needs you: CI failing on an open PR.
  chat({ id: 'oauth', title: 'GitHub OAuth device flow', agent_driver: 'codex', model: 'gpt-5-codex', created_at: ago(DAY + 200), updated_at: ago(55),
    worktree_branch: 'issue-198-device-flow', worktree_status: 'unmerged', pr: prOf(214, 'GitHub OAuth device flow') }),
  // In progress: running, draft PR.
  chat({ id: 'notch', title: 'Desktop notch polish', created_at: ago(200), updated_at: ago(1), running: true,
    worktree_branch: 'feat/notch-polish', worktree_status: 'unmerged', pr: prOf(216, 'Desktop notch polish', 'draft') }),
  chat({ id: 'notch-review', title: 'Second opinion on the notch', launched_by: 'notch', agent_driver: 'claude-code', created_at: ago(30), updated_at: ago(3), running: true }),
  // In progress: idle branch, no PR yet.
  chat({ id: 'hero', title: 'Hero copy pass', cwd: LANDING, agent_driver: 'opencode', created_at: ago(DAY * 2), updated_at: ago(DAY + 30),
    worktree_branch: 'copy/hero', worktree_status: 'unmerged' }),
  // In review: CI passing, review pending.
  chat({ id: 'recents', title: 'Bring back Recents', created_at: ago(DAY * 2), updated_at: ago(180),
    worktree_branch: 'issue-205-recents', worktree_status: 'unmerged', pr: prOf(209, 'Bring back Recents and use the project switcher') }),
  chat({ id: 'recents-2', title: 'Recents: show project names', agent_driver: 'codex', model: 'gpt-5-codex', created_at: ago(DAY), updated_at: ago(200),
    worktree_branch: 'issue-205-recents', worktree_status: 'unmerged', pr: prOf(209, 'Bring back Recents and use the project switcher') }),
  // In review: CI pending.
  chat({ id: 'pricing', title: 'Pricing page layout', cwd: LANDING, created_at: ago(DAY * 3), updated_at: ago(400),
    worktree_branch: 'feat/pricing', worktree_status: 'unmerged', pr: prOf(41, 'Pricing page layout') }),
  // Ready to merge.
  chat({ id: 'issue-picker', title: '# issue mentions in the composer', agent_driver: 'claude-code', created_at: ago(DAY * 3), updated_at: ago(DAY),
    worktree_branch: 'issue-187-issue-mentions', worktree_status: 'unmerged', pr: prOf(208, 'Budgeted context handoffs and # issue mentions') }),
  // Done.
  chat({ id: 'toolbar', title: 'Pin the sidebar toggle', created_at: ago(DAY * 4), updated_at: ago(DAY * 2),
    worktree_branch: 'feat/toolbar', worktree_status: 'merged', pr: prOf(210, 'Pin the sidebar toggle beside the traffic lights', 'merged', ago(DAY * 2)) }),
  chat({ id: 'dead', title: 'Remove dead delegate code', created_at: ago(DAY * 6), updated_at: ago(DAY * 5), worktree_branch: 'chore/dead-delegate', worktree_status: 'merged' }),
  chat({ id: 'composer', title: 'Try a denser composer', created_at: ago(DAY * 8), updated_at: ago(DAY * 6),
    worktree_branch: 'exp/composer', worktree_status: 'unmerged', pr: prOf(160, 'Denser composer', 'closed', ago(DAY * 6)) }),
  // Plain chats with no branch: not work items, Chat tab only.
  chat({ id: 'q', title: 'Answer the pricing question', updated_at: ago(70) }),
  chat({ id: 'flaky', title: 'Investigate the flaky worktree test', updated_at: ago(DAY * 3), model: 'gpt-5' }),
];


type Fake = {
  body: string;
  checks: [string, 'success' | 'failure' | null][];
  reviews?: [string, 'APPROVED' | 'CHANGES_REQUESTED'][];
  requested?: string[];
  decision?: string | null;
  files: [string, number, number][];
  timeline?: [number, string, string][];
};

const PRS: Record<number, Fake> = {
  214: {
    body: 'On a remote box there is no browser for the OAuth redirect. Offer the device-code flow: show a code, poll until the user approves on github.com.\n\nFixes #198',
    checks: [['rust / test', 'failure'], ['rust / clippy', 'success'], ['frontend / test', 'success']],
    requested: ['damilola'], decision: 'REVIEW_REQUIRED',
    files: [['crates/mira-auth/src/device.rs', 212, 0], ['crates/mira-auth/src/lib.rs', 18, 6], ['crates/mira-server/frontend/src/components/Login.tsx', 64, 21]],
    timeline: [[DAY + 20, 'opened', 'opened this pull request'], [55, 'check_failed', 'rust / test failed']],
  },
  216: { body: '', checks: [['rust / test', null], ['frontend / test', 'success']], files: [['apps/desktop/src/main/window.ts', 28, 9], ['crates/mira-server/frontend/src/styles.css', 14, 2]] },
  209: {
    body: 'Bring back a Recents shelf: the chats you touched last, across projects, each naming its project.\n\nCloses #205',
    checks: [['rust / test', 'success'], ['rust / clippy', 'success'], ['frontend / test', 'success']],
    requested: ['damilola'], decision: 'REVIEW_REQUIRED',
    files: [['crates/mira-server/frontend/src/components/Sidebar.tsx', 58, 12], ['crates/mira-server/frontend/src/lib/sidebar.ts', 14, 0], ['crates/mira-server/frontend/tests/sidebar.test.mjs', 16, 0]],
    timeline: [[240, 'opened', 'opened this pull request']],
  },
  41: { body: 'New pricing layout with three plan cards.', checks: [['build', null], ['lint', 'success']], requested: ['damilola'], files: [['src/app/pricing/page.tsx', 140, 38], ['src/components/PlanCard.tsx', 66, 0]] },
  208: {
    body: 'Typing # in the composer searches the repo’s issues and PRs and inserts an owner/repo#N chip.\n\nFixes #187',
    checks: [['rust / test', 'success'], ['rust / clippy', 'success'], ['frontend / test', 'success']],
    reviews: [['damilola', 'APPROVED']], decision: 'APPROVED',
    files: [['crates/mira-server/src/github/issues.rs', 148, 0], ['crates/mira-server/frontend/src/components/composer/IssuePalette.tsx', 260, 0], ['crates/mira-server/frontend/src/lib/issueMentions.ts', 112, 0]],
    timeline: [[DAY * 2, 'opened', 'opened this pull request'], [DAY, 'reviewed', 'approved these changes']],
  },
  210: { body: '', checks: [['rust / test', 'success']], reviews: [['damilola', 'APPROVED']], decision: 'APPROVED', files: [['apps/desktop/src/main/window.ts', 34, 12]] },
  160: { body: '', checks: [], files: [['src/components/Composer.tsx', 40, 60]] },
};

function prDetail(number: number): PullRequestDetailView | null {
  const fake = PRS[number];
  const session = SESSIONS.find((s) => s.pr?.number === number);
  if (!fake || !session?.pr) return null;
  const iso = (mins: number) => new Date((now - mins * 60) * 1000).toISOString();
  const files = fake.files;
  const checks: CheckRunView[] = fake.checks.map(([name, conclusion]) => ({ name, status: conclusion ? 'completed' : 'in_progress', conclusion, url: null }));
  const reviews: ReviewView[] = (fake.reviews ?? []).map(([author, state]) => ({ author, author_avatar: null, state, body: '', submitted_at: iso(DAY) }));
  return {
    summary: {
      number, title: session.pr.title, state: session.pr.state === 'merged' ? 'closed' : session.pr.state, draft: session.pr.state === 'draft',
      author: 'damilola', author_avatar: null, head_ref: session.worktree_branch ?? '', base_ref: 'main', html_url: session.pr.url,
      created_at: iso(DAY * 2), updated_at: iso(30),
      additions: files.reduce((n, f) => n + f[1], 0), deletions: files.reduce((n, f) => n + f[2], 0), changed_files: files.length,
      review_decision: fake.decision ?? null, comment_count: 0, requested_reviewers: fake.requested ?? [],
    },
    body: fake.body, mergeable: true, mergeable_state: 'clean', merged: session.pr.state === 'merged',
    check_status: null, checks, reviews, comments: [], commits: [],
    timeline: (fake.timeline ?? []).map(([mins, kind, message]) => ({ kind, actor: 'damilola', message, at: iso(mins) })),
  };
}

const ISSUES: IssueOption[] = [
  { kind: 'issue', number: 221, title: 'Work tab: group chats by what they deliver', state: 'open', author: 'damilola', labels: [{ name: 'sidebar', color: '1d76db' }, { name: 'design', color: 'd876e3' }], updated_at: new Date(Date.now() - 3600e3).toISOString() },
  { kind: 'issue', number: 219, title: 'Scheduled runs: nightly dependency bumps', state: 'open', author: 'damilola', labels: [{ name: 'feature', color: '0e8a16' }], updated_at: new Date(Date.now() - 86400e3).toISOString() },
  { kind: 'issue', number: 217, title: 'Terminal panel loses scrollback on reattach', state: 'open', author: 'damilola', labels: [{ name: 'bug', color: 'd73a4a' }], updated_at: new Date(Date.now() - 2 * 86400e3).toISOString() },
  { kind: 'issue', number: 213, title: 'Show token usage per work item', state: 'open', author: 'damilola', labels: [], updated_at: new Date(Date.now() - 4 * 86400e3).toISOString() },
  { kind: 'pr', number: 211, title: 'Prototype the sidebar’s Work tab', state: 'open', author: 'damilola', labels: [], updated_at: new Date(Date.now() - 600e3).toISOString() },
];

// Start dark, with the class the `dark:` variants key off.
document.documentElement.dataset.theme = 'dark';
document.documentElement.classList.add('dark');

const realFetch = window.fetch.bind(window);
const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
  const url = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url, location.href);
  const path = url.pathname;
  const method = init?.method ?? 'GET';
  const json = (v: unknown, status = 200) => new Response(JSON.stringify(v), { status, headers: { 'content-type': 'application/json' } });
  if (path === '/api/sessions') return json(url.searchParams.get('archived') === 'true' ? [] : SESSIONS);
  const pr = /^\/api\/prs\/[^/]+\/[^/]+\/(\d+)(\/files|\/merge)?$/.exec(path);
  if (pr) {
    const number = Number(pr[1]);
    await wait(250);
    if (pr[2] === '/merge' && method === 'PUT') {
      for (const s of SESSIONS) if (s.pr?.number === number) s.pr = { ...s.pr, state: 'merged', closed_at: Math.floor(Date.now() / 1000) };
      return json({});
    }
    if (pr[2] === '/files') return json({ files: (PRS[number]?.files ?? []).map(([filename, additions, deletions]) => ({ filename, status: 'modified', additions, deletions, patch: null, raw_url: null })) });
    const detail = prDetail(number);
    return detail ? json(detail) : json({ error: 'not found' }, 404);
  }
  if (path === '/api/git/session-diff') {
    return json({ added: 31, removed: 4, files: [{ path: 'crates/billing/src/webhooks.rs', status: 'modified', added: 27, removed: 4, binary: false }, { path: 'docs/billing.md', status: 'added', added: 4, removed: 0, binary: false }] });
  }
  if (path === '/api/git/branch') return json({ name: url.searchParams.get('name'), base: 'main', ahead: 6, behind: 2, last_subject: 'Persist payments by event id', last_date: ago(40) });
  if (path === '/api/issues') {
    await wait(300);
    const q = (url.searchParams.get('q') ?? '').toLowerCase();
    return json({ owner: 'runmira', repo: 'mira', error: null, items: ISSUES.filter((i) => !q || String(i.number).startsWith(q) || i.title.toLowerCase().includes(q)) });
  }
  if (path === '/api/git/worktree' && method === 'POST') {
    await wait(600);
    const { branch } = JSON.parse(String(init?.body ?? '{}'));
    return json({ path: `${WT}/${branch}`, branch });
  }
  if (path === '/api/cwd' && method === 'PUT') {
    const { path: cwd } = JSON.parse(String(init?.body ?? '{}'));
    const branch = cwd.split('/').pop();
    const id = `chat-${branch}`;
    const at = Math.floor(Date.now() / 1000);
    SESSIONS.push(chat({ id, title: '', message_count: 0, cwd, created_at: at, updated_at: at, worktree_branch: branch, worktree_status: 'unmerged' }));
    return json({ session_id: id });
  }
  const rename = /^\/api\/sessions\/([^/]+)\/title$/.exec(path);
  if (rename && method === 'PATCH') {
    const session = SESSIONS.find((s) => s.id === decodeURIComponent(rename[1]));
    if (session) session.title = JSON.parse(String(init?.body ?? '{}')).title;
    return json({ id: session?.id, title: session?.title });
  }
  if (path.startsWith('/api/')) return json({});
  return realFetch(input, init);
};

function Preview() {
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const view: MainView = pathname.startsWith('/work') ? 'work' : 'chat';
  const [active, setActive] = useState('stripe');
  const openChat = (id: string) => {
    setActive(id);
    void navigate('/chat');
  };
  return (
    <main className="flex h-screen bg-background text-foreground">
      <div className="h-full w-[290px] shrink-0 border-r border-border bg-panel">
        <Sidebar
          status="open"
          cwd={MIRA}
          activeSessionId={active}
          activeBusy={false}
          runningSessions={null}
          completedSessions={new Map()}
          refreshKey={0}
          activeView={view}
          onNavigate={(next) => void navigate(`/${next}`)}
          onNewChat={() => {}}
          onOpenSettings={() => {}}
          onOpenPicker={() => {}}
          onSessionLoaded={() => {}}
          onAttachSession={setActive}
        />
      </div>
      <section className="min-w-0 flex-1">
        {view === 'work' ? (
          <WorkView activeSessionId={active} onOpenChat={openChat} />
        ) : (
          <div className="p-8 text-[13px] text-muted-foreground">
            <h1 className="mb-3 text-[15px] font-semibold text-foreground">Chat</h1>
            <p>Open chat: <b className="text-foreground">{active}</b>. Switch the sidebar to <b>Work</b> to see deliverables.</p>
          </div>
        )}
      </section>
      <button
        type="button"
        className="fixed bottom-3 right-3 rounded-full border border-border bg-panel px-3 py-1 text-[12px] text-muted-foreground hover:text-foreground"
        onClick={() => {
          const light = document.documentElement.dataset.theme !== 'light';
          document.documentElement.dataset.theme = light ? 'light' : 'dark';
          document.documentElement.classList.toggle('dark', !light);
        }}
      >
        Toggle light / dark
      </button>
    </main>
  );
}

const router = createMemoryRouter([{ path: '*', Component: Preview }], { initialEntries: ['/work'] });

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <LazyMotion features={domMax}>
      <RuntimeProvider scope={{ backend: window.location.origin, account: 'preview' }}>
        <RouterProvider router={router} />
      </RuntimeProvider>
    </LazyMotion>
  </React.StrictMode>,
);
