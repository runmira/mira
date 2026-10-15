import { RuntimeProvider } from './state/RuntimeProvider';
/**
 * Dev-only prototype of the sidebar's Work tab. Chat is organised by
 * conversation; Work by deliverable: an issue / branch / PR, with every
 * chat, fork, launched thread and agent that touched it. Items are derived
 * from fake SessionSummary rows by `groupWorkItems`; checks, reviews,
 * plans and diffs are faked here too. Nothing reaches a server.
 * Open /work-preview.html on the Vite dev server.
 */
import { LazyMotion, domMax } from 'framer-motion';
import React, { useState } from 'react';
import ReactDOM from 'react-dom/client';
import './styles.css';
import { Sidebar, type MainView } from './components/Sidebar';
import { branchFor, StartWork } from './components/work/StartWork';
import type { WorkDetail } from './components/work/types';
import { WorkItemPage } from './components/work/WorkItemPage';
import { WorkSidebar } from './components/work/WorkSidebar';
import type { IssueOption } from './lib/issueMentions';
import { groupWorkItems, type WorkMeta } from './lib/workItems';
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

const INITIAL: SessionSummary[] = [
  // Needs you: an approval on the lead chat, a thread still running.
  chat({ id: 'stripe', title: 'Add Stripe webhooks', agent_driver: 'claude-code', created_at: ago(300), updated_at: ago(6),
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

const key = (cwd: string, ref: string | number) => (typeof ref === 'number' ? `${cwd}#${ref}` : `${cwd}@${ref}`);

const META: Record<string, WorkMeta> = {
  [key(MIRA, 'issue-212-stripe-webhooks')]: { issue: { number: 212, title: 'Record Stripe payments from webhooks' } },
  [key(MIRA, 214)]: { issue: { number: 198, title: 'Sign in with GitHub on headless machines' }, ci: 'failing', review: 'pending' },
  [key(MIRA, 216)]: { ci: 'pending' },
  [key(MIRA, 209)]: { issue: { number: 205, title: 'Recents shelf in the sidebar' }, ci: 'passing', review: 'pending' },
  [key(LANDING, 41)]: { ci: 'pending', review: 'pending' },
  [key(MIRA, 208)]: { issue: { number: 187, title: '# to mention issues in the composer' }, ci: 'passing', review: 'approved' },
  [key(MIRA, 210)]: { ci: 'passing', review: 'approved' },
};

const files = (...rows: [string, number, number][]) => rows.map(([path, added, removed]) => ({ path, added, removed }));
const checks = (...rows: [string, 'passing' | 'failing' | 'pending', string?][]) => rows.map(([name, state, duration]) => ({ name, state, duration }));

const DETAILS: Record<string, WorkDetail> = {
  [key(MIRA, 'issue-212-stripe-webhooks')]: {
    goal: 'Payments made through Checkout never reach our database. Add a webhook endpoint that verifies Stripe signatures, records `checkout.session.completed`, and is safe to replay.',
    plan: [
      { text: 'Add /webhooks/stripe route with signature check', done: true },
      { text: 'Persist payments idempotently by event id', done: true },
      { text: 'Retry with backoff on DB errors', done: false },
      { text: 'Document the endpoint and the signing secret', done: false },
    ],
    files: files(['crates/billing/src/webhooks.rs', 186, 4], ['crates/billing/src/store.rs', 42, 11], ['crates/billing/migrations/0007_payments.sql', 24, 0], ['docs/billing.md', 31, 2]),
    worktree: `${WT}/issue-212-stripe-webhooks`, base: 'main', ahead: 6, behind: 2,
    checks: [], reviewers: [],
    timeline: [
      { at: ago(300), text: 'Started from issue #212', kind: 'issue' },
      { at: ago(290), text: 'Claude Code made a plan (4 steps)', kind: 'agent' },
      { at: ago(150), text: '3 commits: route, signature check, store', kind: 'commit' },
      { at: ago(120), text: 'Launched “Docs for the webhook endpoint”', kind: 'chat' },
      { at: ago(90), text: 'You forked to try idempotency keys', kind: 'you' },
      { at: ago(6), text: 'Waiting for you: approve 3 commands', kind: 'agent' },
    ],
  },
  [key(MIRA, 214)]: {
    goal: 'On a remote box there is no browser for the OAuth redirect. Offer the device-code flow: show a code, poll until the user approves on github.com.',
    plan: [
      { text: 'Request a device code', done: true },
      { text: 'Poll the token endpoint with the server-given interval', done: true },
      { text: 'Show the code in the TUI and web', done: true },
      { text: 'Fix slow_down handling', done: false },
    ],
    files: files(['crates/mira-auth/src/device.rs', 212, 0], ['crates/mira-auth/src/lib.rs', 18, 6], ['crates/mira-server/frontend/src/components/Login.tsx', 64, 21]),
    worktree: `${WT}/issue-198-device-flow`, base: 'main', ahead: 4, behind: 0,
    checks: checks(['rust / test', 'failing', '6m 12s'], ['rust / clippy', 'passing', '2m 40s'], ['frontend / test', 'passing', '58s']),
    reviewers: [{ name: 'damilola', state: 'pending' }],
    timeline: [
      { at: ago(DAY + 200), text: 'Started from issue #198', kind: 'issue' },
      { at: ago(DAY + 20), text: 'Codex opened PR #214', kind: 'pr' },
      { at: ago(55), text: 'rust / test failed: device::slow_down_backs_off', kind: 'ci' },
    ],
  },
  [key(MIRA, 216)]: {
    plan: [
      { text: 'Match the notch height to the toolbar', done: true },
      { text: 'Carry the divider through the ledge', done: false },
      { text: 'Check with the sidebar hidden', done: false },
    ],
    files: files(['apps/desktop/src/main/window.ts', 28, 9], ['crates/mira-server/frontend/src/components/PageToolbar.tsx', 6, 3], ['crates/mira-server/frontend/src/styles.css', 14, 2]),
    worktree: `${WT}/feat-notch-polish`, base: 'main', ahead: 2, behind: 0,
    checks: checks(['rust / test', 'pending'], ['frontend / test', 'passing', '1m 02s']),
    reviewers: [],
    timeline: [
      { at: ago(200), text: 'Chat started on feat/notch-polish', kind: 'chat' },
      { at: ago(60), text: 'Draft PR #216 opened', kind: 'pr' },
      { at: ago(30), text: 'Launched a second opinion with Claude Code', kind: 'chat' },
    ],
  },
  [key(LANDING, 'copy/hero')]: {
    plan: [{ text: 'Rewrite the headline', done: true }, { text: 'Tighten the subhead', done: false }],
    files: files(['src/app/page.tsx', 12, 9]),
    worktree: `${LANDING}/.mira/worktrees/copy-hero`, base: 'main', ahead: 1, behind: 5,
    checks: [], reviewers: [],
    timeline: [{ at: ago(DAY * 2), text: 'OpenCode chat started', kind: 'chat' }, { at: ago(DAY + 30), text: '1 commit: headline', kind: 'commit' }],
  },
  [key(MIRA, 209)]: {
    goal: 'Bring back a Recents shelf: the chats you touched last, across projects, each naming its project.',
    plan: [{ text: 'recentChats() with a node test', done: true }, { text: 'Shelf under the nav', done: true }, { text: 'Project name on each row', done: true }],
    files: files(['crates/mira-server/frontend/src/components/Sidebar.tsx', 58, 12], ['crates/mira-server/frontend/src/lib/sidebar.ts', 14, 0], ['crates/mira-server/frontend/tests/sidebar.test.mjs', 16, 0], ['crates/mira-server/frontend/src/components/sidebar/sessionRows.tsx', 22, 6]),
    worktree: `${WT}/issue-205-recents`, base: 'main', ahead: 3, behind: 1,
    checks: checks(['rust / test', 'passing', '5m 50s'], ['rust / clippy', 'passing', '2m 31s'], ['frontend / test', 'passing', '55s']),
    reviewers: [{ name: 'damilola', state: 'pending' }],
    timeline: [
      { at: ago(DAY * 2), text: 'Started from issue #205', kind: 'issue' },
      { at: ago(DAY), text: 'Codex chat joined the branch', kind: 'chat' },
      { at: ago(240), text: 'PR #209 opened', kind: 'pr' },
      { at: ago(200), text: 'All checks passed', kind: 'ci' },
    ],
  },
  [key(LANDING, 41)]: {
    plan: [], files: files(['src/app/pricing/page.tsx', 140, 38], ['src/components/PlanCard.tsx', 66, 0]),
    worktree: `${LANDING}/.mira/worktrees/feat-pricing`, base: 'main', ahead: 2, behind: 0,
    checks: checks(['build', 'pending'], ['lint', 'passing', '22s']),
    reviewers: [{ name: 'damilola', state: 'pending' }],
    timeline: [{ at: ago(DAY * 3), text: 'Chat started', kind: 'chat' }, { at: ago(400), text: 'PR #41 opened', kind: 'pr' }],
  },
  [key(MIRA, 208)]: {
    goal: 'Typing # in the composer should search the repo’s issues and PRs and insert an owner/repo#N chip.',
    plan: [{ text: 'Issue search endpoint', done: true }, { text: '# picker in the composer', done: true }, { text: 'Chips render as GitHub refs', done: true }],
    files: files(['crates/mira-server/src/github/issues.rs', 148, 0], ['crates/mira-server/frontend/src/components/composer/IssuePalette.tsx', 260, 0], ['crates/mira-server/frontend/src/lib/issueMentions.ts', 112, 0], ['crates/mira-server/frontend/src/components/MentionInput.tsx', 40, 8]),
    worktree: `${WT}/issue-187-issue-mentions`, base: 'main', ahead: 5, behind: 0,
    checks: checks(['rust / test', 'passing', '6m 01s'], ['rust / clippy', 'passing', '2m 44s'], ['frontend / test', 'passing', '57s']),
    reviewers: [{ name: 'damilola', state: 'approved' }],
    timeline: [
      { at: ago(DAY * 3), text: 'Started from issue #187', kind: 'issue' },
      { at: ago(DAY * 2), text: 'PR #208 opened', kind: 'pr' },
      { at: ago(DAY + 60), text: 'All checks passed', kind: 'ci' },
      { at: ago(DAY), text: 'damilola approved', kind: 'review' },
    ],
  },
  [key(MIRA, 210)]: {
    plan: [], files: files(['apps/desktop/src/main/window.ts', 34, 12], ['crates/mira-server/frontend/src/components/Sidebar.tsx', 10, 22]),
    worktree: `${WT}/feat-toolbar`, base: 'main',
    checks: checks(['rust / test', 'passing', '5m 44s']), reviewers: [{ name: 'damilola', state: 'approved' }],
    timeline: [{ at: ago(DAY * 2), text: 'Merged into main', kind: 'pr' }],
  },
};

const ISSUES: IssueOption[] = [
  { kind: 'issue', number: 221, title: 'Work tab: group chats by what they deliver', state: 'open', author: 'damilola', labels: [{ name: 'sidebar', color: '1d76db' }, { name: 'design', color: 'd876e3' }], updated_at: new Date(Date.now() - 3600e3).toISOString() },
  { kind: 'issue', number: 219, title: 'Scheduled runs: nightly dependency bumps', state: 'open', author: 'damilola', labels: [{ name: 'feature', color: '0e8a16' }], updated_at: new Date(Date.now() - 86400e3).toISOString() },
  { kind: 'issue', number: 217, title: 'Terminal panel loses scrollback on reattach', state: 'open', author: 'damilola', labels: [{ name: 'bug', color: 'd73a4a' }], updated_at: new Date(Date.now() - 2 * 86400e3).toISOString() },
  { kind: 'issue', number: 213, title: 'Show token usage per work item', state: 'open', author: 'damilola', labels: [], updated_at: new Date(Date.now() - 4 * 86400e3).toISOString() },
  { kind: 'issue', number: 202, title: 'Windows: worktree paths with spaces', state: 'open', author: 'damilola', labels: [{ name: 'windows', color: 'fbca04' }, { name: 'bug', color: 'd73a4a' }], updated_at: new Date(Date.now() - 9 * 86400e3).toISOString() },
];

// Start dark, with the class the `dark:` variants key off.
document.documentElement.dataset.theme = 'dark';
document.documentElement.classList.add('dark');

// The Chat tab still lists every chat, from this page's fake API.
let sessionsForApi = INITIAL;
const realFetch = window.fetch.bind(window);
window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
  const url = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url, location.href);
  const json = (v: unknown) => new Response(JSON.stringify(v), { headers: { 'content-type': 'application/json' } });
  if (url.pathname === '/api/sessions') return json(url.searchParams.get('archived') === 'true' ? [] : sessionsForApi);
  if (/^\/api\/sessions\/[^/]+\/preview$/.test(url.pathname)) {
    return json({ last_user: 'Can you take a look at this?', last_assistant: 'Done — the change is in place.', last_tool: 'bash', message_count: 6 });
  }
  if (url.pathname.startsWith('/api/')) return json({});
  return realFetch(input, init);
};

function Preview() {
  const [view, setView] = useState<MainView>('chat');
  const [workOpen, setWorkOpen] = useState(true);
  const [sessions, setSessions] = useState(INITIAL);
  const [meta, setMeta] = useState(META);
  const [details, setDetails] = useState(DETAILS);
  const items = groupWorkItems(sessions, meta);
  const [activeKey, setActiveKey] = useState<string | null>(key(MIRA, 'issue-212-stripe-webhooks'));
  const [activeChat, setActiveChat] = useState<Record<string, string>>({});
  const [starting, setStarting] = useState(false);
  const item = items.find((i) => i.key === activeKey) ?? null;
  const focusChat = item
    ? (activeChat[item.key] ?? (item.sessions.find((s) => s.needs_attention) ?? item.sessions.find((s) => s.running) ?? item.sessions[0]).id)
    : undefined;

  return (
    <main className="flex h-screen bg-background text-foreground">
      <div className="h-full w-[290px] shrink-0 border-r border-border bg-panel">
        <Sidebar
          status="open"
          cwd={MIRA}
          activeSessionId={focusChat ?? 'stripe'}
          activeBusy={false}
          runningSessions={new Set(sessions.filter((s) => s.running).map((s) => s.id))}
          completedSessions={new Map()}
          refreshKey={0}
          activeView={view}
          onNavigate={setView}
          onNewChat={() => {}}
          onOpenSettings={() => {}}
          onOpenPicker={() => {}}
          onSessionLoaded={() => {}}
          onAttachSession={() => {}}
          work={{
            open: workOpen,
            onToggle: setWorkOpen,
            body: (
              <WorkSidebar
                items={items}
                details={details}
                activeKey={activeKey}
                onPick={setActiveKey}
                onStartWork={() => setStarting(true)}
              />
            ),
          }}
        />
      </div>
      <section className="min-w-0 flex-1">
        {workOpen && item ? (
          <WorkItemPage
            item={item}
            detail={details[item.key]}
            activeSessionId={focusChat}
            onOpenChat={(id) => setActiveChat((m) => ({ ...m, [item.key]: id }))}
            onMerge={() => {
              const at = Math.floor(Date.now() / 1000);
              setSessions((rows) => rows.map((s) => (s.pr?.number === item.pr?.number && s.cwd === item.cwd ? { ...s, pr: { ...s.pr!, state: 'merged', closed_at: at } } : s)));
              const d = details[item.key];
              if (d) setDetails({ ...details, [item.key]: { ...d, timeline: [...d.timeline, { at, text: 'You merged it into main', kind: 'you' }] } });
            }}
          />
        ) : (
          <div className="p-8 text-[13px] text-muted-foreground">
            <h1 className="mb-3 text-[15px] font-semibold text-foreground">Work tab prototype (fake data)</h1>
            <p>Switch the sidebar to <b>Work</b>, then pick an item.</p>
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
      <StartWork
        open={starting}
        onOpenChange={setStarting}
        repo="runmira/mira"
        issues={ISSUES.filter((i) => !items.some((w) => w.meta.issue?.number === i.number))}
        agents={['claude-code', 'codex', 'opencode']}
        onStart={(issue, agent) => {
          const branch = branchFor(issue);
          const id = `work-${issue.number}`;
          const k = key(MIRA, branch);
          const at = Math.floor(Date.now() / 1000);
          const next = [...sessions, chat({ id, title: issue.title, agent_driver: agent, created_at: at, updated_at: at, running: true, worktree_branch: branch, worktree_status: 'unmerged' })];
          sessionsForApi = next;
          setSessions(next);
          setMeta((m) => ({ ...m, [k]: { issue: { number: issue.number, title: issue.title } } }));
          setDetails((d) => ({
            ...d,
            [k]: {
              goal: `${issue.title}.`,
              plan: [{ text: 'Read the issue and the code it touches', done: false }],
              files: [], worktree: `${WT}/${branch}`, base: 'main', ahead: 0, behind: 0,
              checks: [], reviewers: [],
              timeline: [{ at, text: `Started from issue #${issue.number}`, kind: 'issue' }],
            },
          }));
          setActiveKey(k);
        }}
      />
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
