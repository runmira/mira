import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';

const source = ts.transpileModule(readFileSync(new URL('../src/lib/workItems.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.ESNext },
}).outputText;
const { groupWorkItems, workShelves, workState } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);

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
