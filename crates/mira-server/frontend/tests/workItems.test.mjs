import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';

const source = ts.transpileModule(readFileSync(new URL('../src/lib/workItems.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
}).outputText;
const { groupWorkItems, workShelves, workState, prRef, ciState, reviewState, linkedIssue, workPath, branchFor, workKeyFromPath } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);

const chat = (id, extra = {}) => ({ id, cwd: '/work/mira', title: id, created_at: 100, updated_at: 100, message_count: 3, ...extra });
const pr = (number, state = 'open') => ({ number, title: `PR ${number}`, url: '', state });

test('chats on one branch or PR make one item; forks and threads without a branch join their parent', () => {
  const items = groupWorkItems([
    chat('a', { worktree_branch: 'feat/x', pr: pr(1), created_at: 10 }),
    chat('b', { worktree_branch: 'feat/x', pr: pr(1), created_at: 20, agent_driver: 'codex' }),
    chat('fork', { forked_from: 'a', created_at: 30 }),
    chat('thread', { launched_by: 'fork', created_at: 40, agent_driver: 'claude-code' }),
    chat('own', { launched_by: 'a', worktree_branch: 'feat/y' }),
    chat('loose'),
  ]);
  assert.deepEqual(items.map((i) => i.key).sort(), ['/work/mira#1', '/work/mira@feat/y']);
  const x = items.find((i) => i.key === '/work/mira#1');
  assert.deepEqual(x.sessions.map((s) => s.id), ['a', 'b', 'fork', 'thread']);
  assert.deepEqual(x.agents, [null, 'codex', 'claude-code']);
  assert.equal(x.title, 'PR 1');
});

test('worktree chats count as their project, and the item knows the worktree', () => {
  const items = groupWorkItems([
    chat('main', { worktree_branch: 'feat/x' }),
    chat('wt', { worktree_branch: 'feat/x', cwd: '/work/mira/.mira/worktrees/feat-x' }),
  ]);
  assert.equal(items.length, 1);
  assert.equal(items[0].cwd, '/work/mira');
  assert.equal(items[0].worktree, '/work/mira/.mira/worktrees/feat-x');
});

test('the same branch name in two projects is two items, and cycles do not hang', () => {
  const items = groupWorkItems([
    chat('a', { worktree_branch: 'main-fix' }),
    chat('b', { worktree_branch: 'main-fix', cwd: '/work/landing' }),
    chat('c', { forked_from: 'd' }),
    chat('d', { forked_from: 'c' }),
  ]);
  assert.equal(items.length, 2);
});

test('state: waiting beats everything, then done, running, and PR checks', () => {
  const s = (extra) => [chat('a', extra)];
  assert.deepEqual(workState(s({ running: true, needs_attention: true, attention_reason: 'Approve 2 commands' }), pr(1)),
    { state: 'needs-you', reason: 'Approve 2 commands' });
  assert.equal(workState(s({ failure_reason: 'boom' }), null).reason, 'Needs a retry');
  assert.equal(workState(s({}), pr(1, 'merged')).state, 'done');
  assert.equal(workState(s({ worktree_status: 'merged' }), null).state, 'done');
  assert.equal(workState(s({ settled_at: 200 }), null).state, 'done');
  assert.equal(workState(s({ running: true }), pr(1)).state, 'in-progress');
  assert.equal(workState(s({}), pr(1, 'draft')).state, 'in-progress');
  assert.equal(workState(s({}), null).state, 'in-progress');
  assert.equal(workState(s({}), pr(1)).state, 'review');
  assert.equal(workState(s({}), pr(1), { ci: 'passing', review: 'pending' }).state, 'review');
  assert.equal(workState(s({}), pr(1), { ci: 'passing', review: 'approved' }).state, 'ready');
  assert.deepEqual(workState(s({}), pr(1), { ci: 'failing' }), { state: 'needs-you', reason: 'Checks failing' });
});

test('issue titles win, and shelves come in a fixed order', () => {
  const items = groupWorkItems([chat('a', { worktree_branch: 'feat/x' })], { '/work/mira@feat/x': { issue: { number: 7, title: 'Make it work' } } });
  assert.equal(items[0].title, 'Make it work');
  assert.deepEqual(workShelves(items).map(([state, list]) => [state, list.length]),
    [['needs-you', 0], ['in-progress', 1], ['review', 0], ['ready', 0], ['done', 0]]);
});

test('PR refs come from the GitHub URL', () => {
  assert.deepEqual(prRef({ url: 'https://github.com/runmira/mira/pull/211', number: 211 }), { owner: 'runmira', repo: 'mira', number: 211 });
  assert.equal(prRef({ url: '', number: 1 }), null);
});

test('CI: any failure fails, unfinished is pending, none says nothing', () => {
  const run = (status, conclusion = null) => ({ status, conclusion });
  assert.equal(ciState([]), null);
  assert.equal(ciState([run('completed', 'success'), run('completed', 'skipped')]), 'passing');
  assert.equal(ciState([run('completed', 'success'), run('in_progress')]), 'pending');
  assert.equal(ciState([run('in_progress'), run('completed', 'failure')]), 'failing');
});

test('review: the decision wins, else each reviewer\'s latest verdict', () => {
  assert.equal(reviewState('APPROVED'), 'approved');
  assert.equal(reviewState('REVIEW_REQUIRED', [{ author: 'a', state: 'APPROVED' }]), 'pending');
  assert.equal(reviewState(null, [{ author: 'a', state: 'CHANGES_REQUESTED' }, { author: 'a', state: 'APPROVED' }]), 'approved');
  assert.equal(reviewState(null, [{ author: 'a', state: 'APPROVED' }, { author: 'b', state: 'CHANGES_REQUESTED' }]), 'changes-requested');
  assert.equal(reviewState(null, [{ author: 'a', state: 'COMMENTED' }]), null);
});

test('linked issues from the PR body or the branch name', () => {
  assert.equal(linkedIssue('feat/x', 'Adds it.\n\nFixes #42'), 42);
  assert.equal(linkedIssue('issue-205-recents'), 205);
  assert.equal(linkedIssue('feat/198-device-flow'), 198);
  assert.equal(linkedIssue('feat/notch-polish'), null);
  assert.equal(linkedIssue('v2-cleanup'), null);
});

test('work paths survive keys with slashes and #', () => {
  const key = '/Users/me/mira#211';
  assert.equal(decodeURIComponent(workPath(key).slice('/work/'.length)), key);
  assert.equal(workPath(null), '/work');
  assert.equal(workKeyFromPath(workPath(key)), key);
  assert.equal(workKeyFromPath('/work'), null);
  assert.equal(workKeyFromPath('/chat'), null);
  assert.equal(workKeyFromPath('/work/%E0%A4%A'), null);
});

test('issue branches are short, slugged, and link back to the issue', () => {
  assert.equal(branchFor({ number: 217, title: 'Terminal panel loses scrollback on reattach' }), 'issue-217-terminal-panel-loses-scrollback');
  assert.equal(branchFor({ number: 3, title: '🚀 !!' }), 'issue-3');
  assert.equal(linkedIssue(branchFor({ number: 42, title: 'Fix it' })), 42);
});

test('chats in a normal checkout group by the branch they recorded, and its PR', () => {
  const items = groupWorkItems([
    chat('a', { branch: 'feat/205-context', created_at: 10 }),
    chat('b', { branch: 'feat/205-context', branch_pr: pr(208), created_at: 20 }),
    chat('c', { branch: 'feat/other' }),
    chat('wt', { worktree_branch: 'issue-7-fix', branch: 'ignored', cwd: '/work/mira/.mira/worktrees/issue-7-fix' }),
  ]);
  assert.deepEqual(items.map((i) => i.key).sort(), ['/work/mira#208', '/work/mira@feat/other', '/work/mira@issue-7-fix']);
  const ctx = items.find((i) => i.key === '/work/mira#208');
  assert.deepEqual(ctx.sessions.map((s) => s.id), ['a', 'b']);
  assert.equal(ctx.branch, 'feat/205-context');
  assert.equal(linkedIssue(items.find((i) => i.key === '/work/mira#208').branch), 205);
});
